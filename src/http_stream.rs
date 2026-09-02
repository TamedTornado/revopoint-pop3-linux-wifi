use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct StreamLimits {
    pub connect_timeout: Duration,
    pub idle_timeout: Duration,
    pub total_timeout: Duration,
    pub max_header_bytes: usize,
    pub max_body_bytes: usize,
}

#[derive(Debug)]
pub struct StreamError(String);

impl Display for StreamError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for StreamError {}

impl From<io::Error> for StreamError {
    fn from(error: io::Error) -> Self {
        Self(error.to_string())
    }
}

fn fail(message: impl Into<String>) -> StreamError {
    StreamError(message.into())
}

pub fn get_chunked(
    address: SocketAddr,
    path: &str,
    limits: StreamLimits,
    mut receive: impl FnMut(&[u8]),
) -> Result<usize, StreamError> {
    if !path.starts_with('/') || path.contains(['\r', '\n']) {
        return Err(fail("HTTP path must be absolute and contain no newlines"));
    }
    if limits.max_header_bytes < 4 || limits.max_body_bytes == 0 {
        return Err(fail("stream byte limits must be non-zero"));
    }

    let started = Instant::now();
    let mut stream = TcpStream::connect_timeout(&address, limits.connect_timeout)?;
    stream.set_write_timeout(Some(limits.idle_timeout))?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nAccept: */*\r\n\r\n"
    )?;
    stream.flush()?;

    let mut buffered = BufferedStream::new(stream, started, limits);
    let header = buffered.read_header()?;
    validate_response_header(&header)?;

    let mut received = 0_usize;
    loop {
        let size_line = buffered.read_crlf_line(128)?;
        let size_text = size_line
            .split(|byte| *byte == b';')
            .next()
            .ok_or_else(|| fail("missing HTTP chunk size"))?;
        let size_text = std::str::from_utf8(size_text)
            .map_err(|_| fail("HTTP chunk size is not ASCII"))?
            .trim();
        let size =
            usize::from_str_radix(size_text, 16).map_err(|_| fail("invalid HTTP chunk size"))?;

        if size == 0 {
            loop {
                if buffered.read_crlf_line(limits.max_header_bytes)?.is_empty() {
                    return Ok(received);
                }
            }
        }
        if received
            .checked_add(size)
            .is_none_or(|total| total > limits.max_body_bytes)
        {
            return Err(fail("HTTP response exceeds configured body limit"));
        }

        buffered.read_exact_chunks(size, &mut receive)?;
        received += size;
        if buffered.read_exact_vec(2)? != b"\r\n" {
            return Err(fail("HTTP chunk payload is missing its CRLF terminator"));
        }
    }
}

fn validate_response_header(header: &[u8]) -> Result<(), StreamError> {
    let header = std::str::from_utf8(header).map_err(|_| fail("HTTP header is not UTF-8"))?;
    let mut lines = header.split("\r\n");
    let status = lines
        .next()
        .ok_or_else(|| fail("HTTP response has no status line"))?;
    let mut status_fields = status.split_whitespace();
    let version = status_fields.next().unwrap_or_default();
    let code = status_fields.next().unwrap_or_default();
    if !version.starts_with("HTTP/") {
        return Err(fail("invalid HTTP status line"));
    }
    if code != "200" {
        return Err(fail(format!("HTTP stream request returned status {code}")));
    }

    let chunked = lines
        .filter_map(|line| line.split_once(':'))
        .any(|(name, value)| {
            name.eq_ignore_ascii_case("transfer-encoding")
                && value
                    .split(',')
                    .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
        });
    if !chunked {
        return Err(fail("HTTP stream response is not chunked"));
    }
    Ok(())
}

struct BufferedStream {
    stream: TcpStream,
    buffer: Vec<u8>,
    offset: usize,
    started: Instant,
    limits: StreamLimits,
}

impl BufferedStream {
    fn new(stream: TcpStream, started: Instant, limits: StreamLimits) -> Self {
        Self {
            stream,
            buffer: Vec::with_capacity(4096),
            offset: 0,
            started,
            limits,
        }
    }

    fn read_header(&mut self) -> Result<Vec<u8>, StreamError> {
        loop {
            if let Some(position) = self
                .available()
                .windows(4)
                .position(|bytes| bytes == b"\r\n\r\n")
            {
                let length = position + 4;
                return self.take(length);
            }
            if self.available().len() >= self.limits.max_header_bytes {
                return Err(fail("HTTP response header exceeds configured limit"));
            }
            self.read_more()?;
        }
    }

    fn read_crlf_line(&mut self, maximum: usize) -> Result<Vec<u8>, StreamError> {
        loop {
            if let Some(position) = self
                .available()
                .windows(2)
                .position(|bytes| bytes == b"\r\n")
            {
                if position > maximum {
                    return Err(fail("HTTP line exceeds configured limit"));
                }
                let line = self.take(position)?;
                let terminator = self.take(2)?;
                debug_assert_eq!(terminator, b"\r\n");
                return Ok(line);
            }
            if self.available().len() > maximum {
                return Err(fail("HTTP line exceeds configured limit"));
            }
            self.read_more()?;
        }
    }

    fn read_exact_vec(&mut self, length: usize) -> Result<Vec<u8>, StreamError> {
        while self.available().len() < length {
            self.read_more()?;
        }
        self.take(length)
    }

    fn read_exact_chunks(
        &mut self,
        mut remaining: usize,
        receive: &mut impl FnMut(&[u8]),
    ) -> Result<(), StreamError> {
        while remaining > 0 {
            if self.available().is_empty() {
                self.read_more()?;
            }
            let count = remaining.min(self.available().len());
            receive(&self.available()[..count]);
            self.offset += count;
            remaining -= count;
            self.compact();
        }
        Ok(())
    }

    fn take(&mut self, length: usize) -> Result<Vec<u8>, StreamError> {
        if self.available().len() < length {
            return Err(fail("internal buffered read underflow"));
        }
        let result = self.available()[..length].to_vec();
        self.offset += length;
        self.compact();
        Ok(result)
    }

    fn available(&self) -> &[u8] {
        &self.buffer[self.offset..]
    }

    fn compact(&mut self) {
        if self.offset == self.buffer.len() {
            self.buffer.clear();
            self.offset = 0;
        } else if self.offset >= 4096 {
            self.buffer.drain(..self.offset);
            self.offset = 0;
        }
    }

    fn read_more(&mut self) -> Result<(), StreamError> {
        let elapsed = self.started.elapsed();
        let remaining = self
            .limits
            .total_timeout
            .checked_sub(elapsed)
            .ok_or_else(|| fail("HTTP stream exceeded total timeout"))?;
        self.stream
            .set_read_timeout(Some(self.limits.idle_timeout.min(remaining)))?;

        let mut block = [0_u8; 4096];
        let count = self.stream.read(&mut block).map_err(|error| {
            if matches!(
                error.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ) {
                fail("HTTP stream timed out while waiting for data")
            } else {
                StreamError::from(error)
            }
        })?;
        if count == 0 {
            return Err(fail("HTTP stream ended unexpectedly"));
        }
        self.buffer.extend_from_slice(&block[..count]);
        Ok(())
    }
}
