use passfd::FdPassingExt;
use std::io::{self, ErrorKind};
use std::os::fd::{AsRawFd, RawFd};
use tokio::io::Interest;
use tokio::net::UnixStream;

pub async fn recv_fd(stream: &UnixStream) -> io::Result<RawFd> {
    loop {
        stream.readable().await?;
        match stream.try_io(Interest::READABLE, || stream.as_raw_fd().recv_fd()) {
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => continue,
            other => return other,
        }
    }
}
