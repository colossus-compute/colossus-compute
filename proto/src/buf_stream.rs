use std::io;
use std::io::{BufRead, IoSlice, IoSliceMut, Read, Write};

struct BufWritterInner<T: ?Sized + Write>(io::BufWriter<T>);

impl<T: ?Sized + Write> Write for BufWritterInner<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        self.0.write_vectored(bufs)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.0.write_all(buf)
    }
}

impl<T: ?Sized + Write + Read> Read for BufWritterInner<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.get_mut().read(buf)
    }

    fn read_vectored(&mut self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        self.0.get_mut().read_vectored(bufs)
    }

    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> io::Result<usize> {
        self.0.get_mut().read_to_end(buf)
    }

    fn read_to_string(&mut self, buf: &mut String) -> io::Result<usize> {
        self.0.get_mut().read_to_string(buf)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        self.0.get_mut().read_exact(buf)
    }
}

pub struct BufStream<T: ?Sized + Read + Write>(io::BufReader<BufWritterInner<T>>);

impl<T: ?Sized + Read + Write> Write for BufStream<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.get_mut().write(buf)
    }

    fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        self.0.get_mut().write_vectored(bufs)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.get_mut().flush()
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.0.get_mut().write_all(buf)
    }
}

impl<T: ?Sized + Read + Write> Read for BufStream<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }

    fn read_vectored(&mut self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        self.0.read_vectored(bufs)
    }

    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> io::Result<usize> {
        self.0.read_to_end(buf)
    }

    fn read_to_string(&mut self, buf: &mut String) -> io::Result<usize> {
        self.0.read_to_string(buf)
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        self.0.read_exact(buf)
    }
}

impl<T: ?Sized + Read + Write> BufRead for BufStream<T> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.0.fill_buf()
    }

    fn consume(&mut self, amount: usize) {
        self.0.consume(amount)
    }

    fn read_until(&mut self, byte: u8, buf: &mut Vec<u8>) -> io::Result<usize> {
        self.0.read_until(byte, buf)
    }

    fn skip_until(&mut self, byte: u8) -> io::Result<usize> {
        self.0.skip_until(byte)
    }

    fn read_line(&mut self, buf: &mut String) -> io::Result<usize> {
        self.0.read_line(buf)
    }
}

impl<T: Read + Write> BufStream<T> {
    pub fn new(stream: T) -> Self {
        Self(io::BufReader::new(BufWritterInner(io::BufWriter::new(stream))))
    }
}