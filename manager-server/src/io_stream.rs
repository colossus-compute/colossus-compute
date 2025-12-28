use std::io;
use std::io::{Error, IoSlice, IoSliceMut, Read, Write};
use std::pin::Pin;
use std::task::{Context, Poll};
use pin_project_lite::pin_project;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pin_project! {
    pub struct Stream<R, W> {
        #[pin]
        read: R,
        #[pin]
        write:  W
    }
}

impl<R, W> Stream<R, W> {
    pub fn new(read: R, write: W) -> Self {
        Self { read, write }
    }
}

impl<R, W: Write> Write for Stream<R, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.write.write(buf)
    }

    fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        self.write.write_vectored(bufs)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.write.flush()
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.write.write_all(buf)
    }
}

impl<R: Read, W> Read for Stream<R, W> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.read.read(buf)
    }

    fn read_vectored(&mut self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        self.read.read_vectored(bufs)
    }

    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> io::Result<usize> {
        self.read.read_to_end(buf)
    }

    fn read_to_string(&mut self, buf: &mut String) -> io::Result<usize> {
        self.read.read_to_string(buf)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        self.read.read_exact(buf)
    }
}