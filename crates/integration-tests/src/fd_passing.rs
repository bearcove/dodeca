use passfd::FdPassingExt;
use std::io::{self, ErrorKind};
use std::os::fd::{AsRawFd, RawFd};
use tokio::io::Interest;
use tokio::net::UnixStream;

pub async fn send_fd(stream: &UnixStream, fd: RawFd) -> io::Result<()> {
    loop {
        stream.writable().await?;
        match stream.try_io(Interest::WRITABLE, || stream.as_raw_fd().send_fd(fd)) {
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => continue,
            other => return other,
        }
    }
}

pub async fn recv_fd(stream: &UnixStream) -> io::Result<RawFd> {
    loop {
        stream.readable().await?;
        match stream.try_io(Interest::READABLE, || stream.as_raw_fd().recv_fd()) {
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => continue,
            other => return other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::IntoRawFd;
    use std::os::unix::net::UnixStream as StdUnixStream;

    #[test]
    fn send_fd_does_not_close_sender_fd() {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

        let (a_std, b_std) = StdUnixStream::pair().expect("unix pair");
        a_std.set_nonblocking(true).expect("nonblocking");
        b_std.set_nonblocking(true).expect("nonblocking");

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("tcp bind");
        let fd = listener.into_raw_fd();

        rt.block_on(async {
            let a = UnixStream::from_std(a_std).expect("tokio unix stream");
            let b = UnixStream::from_std(b_std).expect("tokio unix stream");

            send_fd(&a, fd).await.expect("send fd");
            let received_fd = recv_fd(&b).await.expect("recv fd");

            // If the sender FD got closed, fcntl(F_GETFD) will return -1 with EBADF.
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
            assert_ne!(flags, -1, "sender fd unexpectedly closed");

            unsafe {
                libc::close(fd);
                libc::close(received_fd);
            }
        });
    }
}
