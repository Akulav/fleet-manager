//! Launch and supervise the user-session helper from the LocalSystem service.
//!
//! A service runs in session 0 and cannot show UI to, or capture the screen of,
//! the interactive user. The standard, documented way around this is to grab the
//! active console session's user token and spawn a child process into that
//! session with `CreateProcessAsUserW`. We relaunch the helper whenever it exits
//! or the active session changes (log off / fast user switch / RDP).

use std::time::Duration;

use tokio_util::sync::CancellationToken;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, STILL_ACTIVE};
use windows::Win32::Security::{
    DuplicateTokenEx, SecurityIdentification, TokenPrimary, TOKEN_ALL_ACCESS,
};
use windows::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows::Win32::System::RemoteDesktop::{
    WTSGetActiveConsoleSessionId, WTSQueryUserToken,
};
use windows::Win32::System::Threading::{
    CreateProcessAsUserW, GetExitCodeProcess, CREATE_NEW_CONSOLE, CREATE_UNICODE_ENVIRONMENT,
    PROCESS_INFORMATION, STARTUPINFOW,
};

/// Owns a HANDLE and closes it on drop.
struct OwnedHandle(HANDLE);
// The handles we own (process / token) are used from a single supervisor task.
// Windows HANDLEs are plain kernel handles, safe to move between threads.
unsafe impl Send for OwnedHandle {}
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

const NO_SESSION: u32 = 0xFFFFFFFF;

pub async fn helper_supervisor(hub: crate::ipc::Hub, shutdown: CancellationToken) {
    let mut current: Option<(u32, OwnedHandle)> = None; // (session_id, process handle)

    // Wait until the IPC server has bound a port we can hand to the helper.
    loop {
        if shutdown.is_cancelled() {
            return;
        }
        if hub.port() != 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let ipc_port = hub.port();
    let ipc_token = hub.0.token.clone();

    loop {
        if shutdown.is_cancelled() {
            break;
        }

        let session = unsafe { WTSGetActiveConsoleSessionId() };
        let have_user = session != NO_SESSION && session != 0;

        // Decide whether we need to (re)launch.
        let needs_launch = match &current {
            None => have_user,
            Some((sid, proc)) => {
                let alive = process_alive(proc.0);
                if !alive {
                    tracing::info!("helper exited; will relaunch");
                }
                let session_changed = *sid != session;
                (session_changed || !alive) && have_user
            }
        };

        if needs_launch {
            match launch_helper_in_session(session, ipc_port, &ipc_token) {
                Ok(h) => {
                    tracing::info!("launched helper in session {session}");
                    current = Some((session, h));
                }
                Err(e) => tracing::warn!("could not launch helper in session {session}: {e:#}"),
            }
        } else if !have_user {
            current = None;
        }

        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = tokio::time::sleep(Duration::from_secs(3)) => {}
        }
    }
}

fn process_alive(h: HANDLE) -> bool {
    if h.is_invalid() {
        return false;
    }
    let mut code: u32 = 0;
    unsafe {
        if GetExitCodeProcess(h, &mut code).is_ok() {
            return code == STILL_ACTIVE.0 as u32;
        }
    }
    false
}

fn launch_helper_in_session(
    session_id: u32,
    ipc_port: u16,
    ipc_token: &str,
) -> windows::core::Result<OwnedHandle> {
    unsafe {
        // Token of the user logged into that session.
        let mut user_token = HANDLE::default();
        WTSQueryUserToken(session_id, &mut user_token)?;
        let user_token = OwnedHandle(user_token);

        // Duplicate into a primary token we can launch a process with.
        let mut primary = HANDLE::default();
        DuplicateTokenEx(
            user_token.0,
            TOKEN_ALL_ACCESS,
            None,
            SecurityIdentification,
            TokenPrimary,
            &mut primary,
        )?;
        let primary = OwnedHandle(primary);

        // Build the user's environment block.
        let mut env_block: *mut core::ffi::c_void = std::ptr::null_mut();
        let have_env = CreateEnvironmentBlock(&mut env_block, primary.0, false).is_ok();

        // Command line: "<exe>" helper
        let exe = std::env::current_exe().map_err(|_| windows::core::Error::from_win32())?;
        let mut cmdline: Vec<u16> =
            format!("\"{}\" helper --ipc-port {ipc_port} --ipc-token {ipc_token}", exe.display())
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();

        let mut desktop: Vec<u16> = "winsta0\\default\0".encode_utf16().collect();
        let mut si = STARTUPINFOW::default();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        si.lpDesktop = PWSTR(desktop.as_mut_ptr());

        let mut pi = PROCESS_INFORMATION::default();

        let flags = CREATE_UNICODE_ENVIRONMENT | CREATE_NEW_CONSOLE;
        let env_ptr = if have_env {
            Some(env_block as *const core::ffi::c_void)
        } else {
            None
        };

        let result = CreateProcessAsUserW(
            primary.0,
            PCWSTR::null(),
            PWSTR(cmdline.as_mut_ptr()),
            None,
            None,
            false,
            flags,
            env_ptr,
            PCWSTR::null(),
            &si,
            &mut pi,
        );

        if have_env && !env_block.is_null() {
            let _ = DestroyEnvironmentBlock(env_block);
        }

        result?;

        // We don't need the thread handle.
        if !pi.hThread.is_invalid() {
            let _ = CloseHandle(pi.hThread);
        }
        Ok(OwnedHandle(pi.hProcess))
    }
}
