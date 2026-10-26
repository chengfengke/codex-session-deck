//! Local IPC and process setup without exposing a network listener.
use std::{io, path::Path, process::Command};
use tokio::io::{AsyncRead, AsyncWrite};

pub trait Ipc: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Ipc for T {}
pub type Stream = Box<dyn Ipc>;

pub async fn shutdown_signal() {
    #[cfg(unix)]
    let _ = tokio::signal::ctrl_c().await;
    // A detached Windows daemon has no console to receive Ctrl+C. It is shut
    // down through authenticated local IPC instead of registering a handler.
    #[cfg(windows)]
    std::future::pending::<()>().await;
}

pub fn private_permissions(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    }
    #[cfg(windows)]
    let _ = (path, mode); // Files inherit the user's state-directory ACL.
    Ok(())
}

pub fn detach(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Only an async-signal-safe syscall is made between fork and exec.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Detached process with a new process group; inherited handles are null/log.
        command.creation_flags(0x00000008 | 0x00000200);
    }
}

#[cfg(unix)]
pub struct Listener(tokio::net::UnixListener);
#[cfg(unix)]
impl Listener {
    pub fn bind() -> io::Result<Self> {
        cleanup();
        let listener = tokio::net::UnixListener::bind(crate::model::socket_path())?;
        private_permissions(&crate::model::socket_path(), 0o600)?;
        Ok(Self(listener))
    }
    pub async fn accept(&mut self) -> io::Result<Stream> {
        Ok(Box::new(self.0.accept().await?.0))
    }
}
#[cfg(unix)]
pub async fn connect() -> io::Result<Stream> {
    Ok(Box::new(
        tokio::net::UnixStream::connect(crate::model::socket_path()).await?,
    ))
}
#[cfg(unix)]
pub fn cleanup() {
    let _ = std::fs::remove_file(crate::model::socket_path());
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{ffi::c_void, os::windows::process::CommandExt, ptr};
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};

    #[repr(C)]
    struct SecurityAttributes {
        length: u32,
        descriptor: *mut c_void,
        inherit: i32,
    }
    #[link(name = "advapi32")]
    extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text: *const u16,
            revision: u32,
            descriptor: *mut *mut c_void,
            size: *mut u32,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }

    pub struct Listener {
        pending: NamedPipeServer,
        sddl: Vec<u16>,
    }
    impl Listener {
        pub fn bind() -> io::Result<Self> {
            // An explicit current-user DACL prevents other local accounts from
            // reading snapshots or submitting commands to the pipe.
            let output = Command::new("whoami.exe")
                .args(["/user", "/fo", "csv", "/nh"])
                .creation_flags(0x08000000)
                .output()?;
            if !output.status.success() {
                return Err(io::Error::other(
                    "Cannot identify Windows user for private IPC",
                ));
            }
            let text = String::from_utf8_lossy(&output.stdout);
            let sid = text
                .split(',')
                .map(|s| s.trim().trim_matches('"'))
                .find(|s| {
                    s.starts_with("S-1-")
                        && s.chars()
                            .all(|c| c.is_ascii_digit() || c == '-' || c == 'S')
                })
                .ok_or_else(|| io::Error::other("Cannot read Windows user SID"))?;
            let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})\0").encode_utf16().collect();
            let pending = Self::create(&sddl, true)?;
            Ok(Self { pending, sddl })
        }
        fn create(sddl: &[u16], first: bool) -> io::Result<NamedPipeServer> {
            let mut descriptor = ptr::null_mut();
            // The descriptor and SECURITY_ATTRIBUTES live until CreateNamedPipe
            // returns; Windows copies the security descriptor during creation.
            unsafe {
                if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    1,
                    &mut descriptor,
                    ptr::null_mut(),
                ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                let mut attrs = SecurityAttributes {
                    length: std::mem::size_of::<SecurityAttributes>() as u32,
                    descriptor,
                    inherit: 0,
                };
                let result = ServerOptions::new()
                    .first_pipe_instance(first)
                    .reject_remote_clients(true)
                    .create_with_security_attributes_raw(
                        crate::model::socket_path(),
                        &mut attrs as *mut _ as *mut c_void,
                    );
                LocalFree(descriptor);
                result
            }
        }
        pub async fn accept(&mut self) -> io::Result<Stream> {
            self.pending.connect().await?;
            let next = Self::create(&self.sddl, false)?;
            Ok(Box::new(std::mem::replace(&mut self.pending, next)))
        }
    }
    pub async fn connect() -> io::Result<Stream> {
        Ok(Box::new(
            ClientOptions::new().open(crate::model::socket_path())?,
        ))
    }
}
#[cfg(windows)]
pub use windows::{connect, Listener};
#[cfg(windows)]
pub fn cleanup() {}
