use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags, recvmsg};
use std::{
    io::{self, IoSliceMut},
    net::{Ipv4Addr, SocketAddr},
    os::{fd::OwnedFd, unix::net::UnixStream as StdUnixStream},
    path::Path,
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::Interest,
    net::{TcpListener, TcpStream, UnixStream},
    process::Command,
    task::JoinSet,
};

const HELPER: &str = match option_env!("REXIA_BIND_HELPER") {
    Some(path) => path,
    None => "/usr/libexec/rexia-bind-port",
};
const POLICY: &str = "dev.hackr.rexia.bind-port.policy";

fn pkexec() -> Option<&'static str> {
    ["/run/wrappers/bin/pkexec", "/usr/bin/pkexec"]
        .into_iter()
        .find(|path| Path::new(path).is_file())
}

pub fn available() -> bool {
    if !Path::new(HELPER).is_file() || pkexec().is_none() {
        return false;
    }
    if option_env!("REXIA_BIND_HELPER").is_some() {
        // A Nix build may be runnable without having registered its policy.
        // Use only the policy installed for this exact build, not generic pkexec.
        let Some(package) = Path::new(HELPER).parent().and_then(Path::parent) else {
            return false;
        };
        let relative = Path::new("share/polkit-1/actions").join(POLICY);
        std::fs::canonicalize(Path::new("/run/current-system/sw").join(&relative))
            .is_ok_and(|registered| registered == package.join(relative))
    } else {
        Path::new("/usr/share/polkit-1/actions")
            .join(POLICY)
            .is_file()
    }
}

pub(super) async fn bind(port: u16) -> Result<TcpListener, String> {
    if !available() {
        return Err("The system port authorization helper is not installed".into());
    }
    let mut command = Command::new(pkexec().ok_or("Polkit is unavailable")?);
    command
        // The helper is executed directly; pkexec rejects unlisted inherited shells.
        .env_remove("SHELL")
        .args(["--disable-internal-agent", HELPER, &port.to_string()]);
    tokio::time::timeout(Duration::from_secs(120), receive(command, port))
        .await
        .map_err(|_| "Port authorization timed out".to_owned())?
}

async fn receive(mut command: Command, port: u16) -> Result<TcpListener, String> {
    let (read, child) = (|| -> io::Result<_> {
        let (read, write) = StdUnixStream::pair()?;
        read.set_nonblocking(true)?;
        let read = UnixStream::from_std(read)?;
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::from(OwnedFd::from(write)))
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        Ok((read, child))
    })()
    .map_err(|error| error.to_string())?;
    // Drop the parent's copy of the sending end before waiting for EOF.
    drop(command);
    let (fd, output) = tokio::join!(
        read.async_io(Interest::READABLE, || receive_fd(&read)),
        child.wait_with_output(),
    );
    let output = output.map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(if output.status.code() == Some(126) {
            "Port authorization was canceled".into()
        } else {
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            format!("Could not authorize port {port}: {}", diagnostic.trim())
        });
    }
    let fd = fd.map_err(|error| error.to_string())?;
    if !rustix::net::sockopt::socket_acceptconn(&fd).unwrap_or(false) {
        return Err("The port helper did not return a listening socket".into());
    }
    let listener = std::net::TcpListener::from(fd);
    if listener.local_addr().ok() != Some(SocketAddr::from((Ipv4Addr::LOCALHOST, port))) {
        return Err("The port helper returned the wrong listening address".into());
    }
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    TcpListener::from_std(listener).map_err(|error| error.to_string())
}

fn receive_fd(read: &UnixStream) -> io::Result<OwnedFd> {
    let mut marker = [0];
    let mut space = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut control = RecvAncillaryBuffer::new(&mut space);
    let result = recvmsg(
        read,
        &mut [IoSliceMut::new(&mut marker)],
        &mut control,
        RecvFlags::CMSG_CLOEXEC,
    )?;
    if result.bytes == 1
        && marker == *b"F"
        && !result.flags.contains(ReturnFlags::CTRUNC)
        && let Some(RecvAncillaryMessage::ScmRights(mut fds)) = control.drain().next()
        && let Some(fd) = fds.next()
        && fds.next().is_none()
    {
        return Ok(fd);
    }
    Err(io::Error::other("The port helper did not return a socket"))
}

pub(super) async fn relay(listener: TcpListener, upstream: SocketAddr) -> io::Result<()> {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            _ = connections.join_next(), if !connections.is_empty() => {}
            accepted = listener.accept(), if connections.len() < 128 => {
                let (mut client, _) = accepted?;
                connections.spawn(async move {
                    let mut server = TcpStream::connect(upstream).await?;
                    tokio::io::copy_bidirectional(&mut client, &mut server).await
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn relay_copies_both_directions_and_cancels_active_connections() {
        tokio::time::timeout(Duration::from_secs(3), async {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let local = listener.local_addr().unwrap();
            let upstream = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let task = tokio::spawn(relay(listener, upstream.local_addr().unwrap()));
            let mut client = TcpStream::connect(local).await.unwrap();
            let (mut server, _) = upstream.accept().await.unwrap();
            client.write_all(b"a").await.unwrap();
            client.shutdown().await.unwrap();
            let mut request = Vec::new();
            server.read_to_end(&mut request).await.unwrap();
            assert_eq!(request, b"a");
            server.write_all(b"b").await.unwrap();
            server.shutdown().await.unwrap();
            let mut response = Vec::new();
            client.read_to_end(&mut response).await.unwrap();
            assert_eq!(response, b"b");
            // Leave a second connection open to exercise cancellation of the
            // relay's child tasks, rather than only an already-finished copy.
            let mut active_client = TcpStream::connect(local).await.unwrap();
            let (mut active_server, _) = upstream.accept().await.unwrap();
            active_client.write_all(b"c").await.unwrap();
            let mut byte = [0];
            active_server.read_exact(&mut byte).await.unwrap();
            assert_eq!(&byte, b"c");

            task.abort();
            // Joining observes the relay future being dropped, including its
            // owned listener and JoinSet. Child cancellation may finish later.
            assert!(task.await.unwrap_err().is_cancelled());
            for peer in [&mut active_client, &mut active_server] {
                match peer.read(&mut byte).await {
                    Ok(0) => {}
                    Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
                    result => panic!("relay left an active connection open: {result:?}"),
                }
            }
            // Do not probe `local` with a new connection: once released, the
            // ephemeral port may belong to an unrelated parallel test. Check
            // our established peers instead; their identity cannot be reused.
        })
        .await
        .unwrap();
    }
}
