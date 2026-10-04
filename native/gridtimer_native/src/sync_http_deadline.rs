// v0.0.2 - Interrupt local sockets and connect waits when their supervised task is cancelled.
use super::{SYNC_CONNECT_TIMEOUT, SYNC_RESPONSE_READ_TIMEOUT, SYNC_WRITE_TIMEOUT};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

pub(super) fn remaining(deadline: Instant, idle_timeout: Duration) -> io::Result<Duration> {
    #[cfg(not(target_os = "android"))]
    crate::runtime::cancellation::check_current()?;
    let budget = deadline.saturating_duration_since(Instant::now());
    if budget.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "sync request exceeded its total deadline",
        ));
    }
    Ok(budget.min(idle_timeout))
}

pub(super) fn connect_loopback(host: &str, port: u16, deadline: Instant) -> io::Result<TcpStream> {
    // Only local numeric addresses and the exact localhost alias are accepted.
    // Resolve that alias directly so DNS cannot outlive the request budget or
    // redirect account credentials off-device through a hosts-file override.
    let addresses: Vec<IpAddr> = if host.eq_ignore_ascii_case("localhost") {
        vec![Ipv4Addr::LOCALHOST.into(), Ipv6Addr::LOCALHOST.into()]
    } else {
        match host.parse::<IpAddr>() {
            Ok(address) if address.is_loopback() => vec![address],
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "HTTP peer is not local",
                ))
            }
        }
    };
    let mut last_error = None;
    for address in addresses {
        let attempt_deadline = deadline.min(Instant::now() + SYNC_CONNECT_TIMEOUT);
        loop {
            let timeout = match remaining(attempt_deadline, SYNC_CONNECT_TIMEOUT) {
                Ok(timeout) => timeout,
                Err(error)
                    if error.kind() == io::ErrorKind::TimedOut && Instant::now() < deadline =>
                {
                    last_error = Some(error);
                    break;
                }
                Err(error) => return Err(error),
            };
            #[cfg(not(target_os = "android"))]
            let timeout = if crate::runtime::cancellation::current().is_some() {
                timeout.min(Duration::from_millis(100))
            } else {
                timeout
            };
            match TcpStream::connect_timeout(&SocketAddr::new(address, port), timeout) {
                Ok(stream) => return Ok(stream),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) && Instant::now() < attempt_deadline =>
                {
                    continue
                }
                Err(error) => {
                    last_error = Some(error);
                    break;
                }
            }
        }
    }
    Err(last_error
        .unwrap_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "no local address")))
}

#[cfg(not(target_os = "android"))]
pub(super) fn interrupt_on_cancel(
    stream: &TcpStream,
) -> io::Result<Option<crate::runtime::cancellation::Registration>> {
    crate::runtime::cancellation::current()
        .map(|token| {
            token.check()?;
            let socket = stream.try_clone()?;
            let lease = token.on_cancel(move || {
                let _ = socket.shutdown(std::net::Shutdown::Both);
            });
            token.check()?;
            Ok(lease)
        })
        .transpose()
}

pub(super) struct DeadlineStream {
    stream: TcpStream,
    deadline: Instant,
}

impl DeadlineStream {
    pub(super) fn new(stream: TcpStream, deadline: Instant) -> io::Result<Self> {
        #[cfg(target_os = "windows")]
        stream.set_nonblocking(true)?;
        Ok(Self { stream, deadline })
    }

    #[cfg(target_os = "windows")]
    fn wait_ready(&self, writing: bool, deadline: Instant) -> io::Result<()> {
        use std::os::windows::io::AsRawSocket;
        #[repr(C)]
        struct PollFd {
            socket: usize,
            events: i16,
            returned: i16,
        }
        #[link(name = "ws2_32")]
        unsafe extern "system" {
            fn WSAPoll(sockets: *mut PollFd, count: u32, timeout: i32) -> i32;
            fn WSAGetLastError() -> i32;
        }
        let timeout = remaining(deadline, Duration::MAX)?;
        let timeout = if crate::runtime::cancellation::current().is_some() {
            timeout.min(Duration::from_millis(100))
        } else {
            timeout
        };
        let mut descriptor = PollFd {
            socket: self.stream.as_raw_socket() as usize,
            events: if writing { 0x0010 } else { 0x0100 },
            returned: 0,
        };
        let status = unsafe {
            WSAPoll(
                &mut descriptor,
                1,
                timeout.as_millis().clamp(1, i32::MAX as u128) as i32,
            )
        };
        if status < 0 {
            return Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }));
        }
        if descriptor.returned & 0x0004 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "sync socket is no longer valid",
            ));
        }
        Ok(())
    }
}

impl Read for DeadlineStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        #[cfg(target_os = "windows")]
        {
            let deadline = self
                .deadline
                .min(Instant::now() + SYNC_RESPONSE_READ_TIMEOUT);
            loop {
                remaining(deadline, SYNC_RESPONSE_READ_TIMEOUT)?;
                match self.stream.read(buffer) {
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        self.wait_ready(false, deadline)?
                    }
                    result => return result,
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let timeout = remaining(self.deadline, SYNC_RESPONSE_READ_TIMEOUT)?;
            self.stream.set_read_timeout(Some(timeout))?;
            self.stream.read(buffer)
        }
    }
}

impl Write for DeadlineStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        #[cfg(target_os = "windows")]
        {
            let deadline = self.deadline.min(Instant::now() + SYNC_WRITE_TIMEOUT);
            loop {
                remaining(deadline, SYNC_WRITE_TIMEOUT)?;
                // Only a nonblocking WouldBlock is safe to wait and try again.
                // A blocking Winsock timeout leaves the connection indeterminate.
                match self.stream.write(buffer) {
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        self.wait_ready(true, deadline)?
                    }
                    result => return result,
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let timeout = remaining(self.deadline, SYNC_WRITE_TIMEOUT)?;
            self.stream.set_write_timeout(Some(timeout))?;
            self.stream.write(buffer)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        remaining(self.deadline, SYNC_WRITE_TIMEOUT)?;
        self.stream.flush()
    }
}
