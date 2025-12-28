use tokio::io::AsyncReadExt;
use tokio::io::AsyncBufRead;
extern crate core;

use core::slice;
use std::io;
use std::ops::Range;
use std::io::{BufRead, Read, Write};
use std::num::NonZero;
use crate::limited_slice::{LimitedSlice, LimitedStr};

pub mod buf_stream;
pub mod limited_slice;
pub mod alloc;

fn read_n_bytes<const N: usize, R: ?Sized + BufRead>(reader: &mut R) -> io::Result<[u8; N]> {
    let initial_buffer = reader.fill_buf()?;
    if let Some(&chunk) = initial_buffer.first_chunk::<N>() {
        reader.consume(N);
        return Ok(chunk)
    }

    if initial_buffer.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!("failed to read {N} bytes")
        ))
    }

    let mut chunk = [0; N];
    let (already_read, to_fill) = chunk.split_at_mut(initial_buffer.len());
    already_read.copy_from_slice(initial_buffer);

    reader.read_exact(to_fill)?;

    Ok(chunk)
}

trait BufReadExt: BufRead {
    fn read_u16_le(&mut self) -> io::Result<u16> {
        read_n_bytes::<2, Self>(self).map(u16::from_le_bytes)
    }

    fn read_u32_le(&mut self) -> io::Result<u32> {
        read_n_bytes::<4, Self>(self).map(u32::from_le_bytes)
    }

    fn read_u64_le(&mut self) -> io::Result<u64> {
        read_n_bytes::<8, Self>(self).map(u64::from_le_bytes)
    }

}

impl<R: ?Sized + BufRead> BufReadExt for R {}


trait WriteExt: Write {
    fn write_u16_le(&mut self, int: u16) -> io::Result<()> {
        self.write_all(&u16::to_le_bytes(int))
    }

    fn write_u32_le(&mut self, int: u32) -> io::Result<()> {
        self.write_all(&u32::to_le_bytes(int))
    }

    fn write_u64_le(&mut self, int: u64) -> io::Result<()> {
        self.write_all(&u64::to_le_bytes(int))
    }
}

impl<W: ?Sized + Write> WriteExt for W {}

#[derive(Copy, Clone)]
pub struct Request<'a> {
    name: LimitedStr<'a, u16>,
    start: u64,
    length: u32,
    output_length: u32,
    input: LimitedSlice<'a, u8, u32>,
    wasm_object: LimitedSlice<'a, u8, u32>
}

#[derive(Debug, thiserror::Error)]
pub enum RequestCreationError {
    #[error("name too long")]
    NameTooLong,
    #[error("requested range end overflowed 64 bit integer")]
    OutOfRange,
    #[error("input too big")]
    InputTooBig,
    #[error("wasm object too big")]
    WasmObjectTooBig,
}

impl<'a> Request<'a> {
    pub fn new(
        name: &'a str,
        start: u64,
        length: u32,
        output_length: u32,
        input: &'a [u8],
        wasm_object: &'a [u8]
    ) -> Result<Self, RequestCreationError> {
        let Some(name) = LimitedStr::new(name) else {
            return Err(RequestCreationError::NameTooLong)
        };

        if start.checked_add(u64::from(length)).is_none() {
            return Err(RequestCreationError::OutOfRange)
        }

        let Some(input) = LimitedSlice::new(input) else {
            return Err(RequestCreationError::InputTooBig)
        };

        let Some(wasm_object) = LimitedSlice::new(wasm_object) else {
            return Err(RequestCreationError::WasmObjectTooBig)
        };

        Ok(Self {
            name,
            start,
            length,
            output_length,
            input,
            wasm_object,
        })
    }

    pub fn name(&self) -> &'a str {
        self.name.as_str()
    }

    pub fn start(&self) -> u64 {
        self.start
    }

    pub fn length(&self) -> u32 {
        self.length
    }

    pub fn range(&self) -> Range<u64> {
        let start = self.start;
        let end = unsafe { start.unchecked_add(u64::from(self.length)) };
        start..end
    }


    pub fn input(&self) -> &'a [u8] {
        self.input.as_slice()
    }

    pub fn wasm_object(&self) -> &'a [u8] {
        self.wasm_object.as_slice()
    }

    pub fn output_length(&self) -> u32 {
        self.output_length
    }
}


macro_rules! gen_read_n {
    ($([async] $({$async: ident})?)? $name: ident($(+ $bound:tt)*)) => {
        pub $(async $($async)?)? fn $name<R: ?Sized $(+ $bound )+>(
            buffer: &mut Vec<u8>,
            reader: &mut R,
            len: usize
        ) -> io::Result<Range<usize>> {
            let Ok(len_64) = u64::try_from(len) else {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof))
            };

            buffer.try_reserve(len)?;

            let start = buffer.len();
            let n = reader
                .take(len_64)
                .read_to_end(buffer)
                $(.await $($async)?)??;

            if n != len {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof))
            }
            let end = start + len;
            assert_eq!(end, buffer.len());

            Ok(start..end)
        }
    };
}

gen_read_n!(read_n(+ BufRead));
gen_read_n!([async] read_n_async(+ AsyncBufRead + Unpin));

impl<'a> Request<'a> {
    // TODO: use buffer for bufread
    pub fn read<R: ?Sized + BufRead>(buffer: &'a mut Vec<u8>, reader: &mut R) -> io::Result<Self> {
        buffer.clear();
        let mut read_n = |reader: &mut R, len: usize| read_n(buffer, reader, len);
        let mut read_u16_bounded = |reader: &mut R| {
            let len = reader.read_u16_le()?;
            read_n(reader, usize::from(len))
        };

        let name = read_u16_bounded(reader)?;

        let start = reader.read_u64_le()?;
        let length = reader.read_u32_le()?;
        let output_length = reader.read_u32_le()?;

        if start.checked_add(u64::from(length)).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                RequestCreationError::OutOfRange
            ))
        }

        let mut read_u32_bounded = |reader: &mut R|  {
            let len = reader.read_u32_le()?;
            let Ok(len) = usize::try_from(len) else {
                return Err(io::Error::from(io::ErrorKind::OutOfMemory))
            };

            read_n(reader, len)
        };

        let input = read_u32_bounded(reader)?;
        let wasm_object = read_u32_bounded(reader)?;

        let name = str::from_utf8(&buffer[name])
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

        let this = Self {
            name: LimitedStr::new(name).unwrap(),
            start,
            length,
            output_length,
            input: LimitedSlice::new(&buffer[input]).unwrap(),
            wasm_object: LimitedSlice::new(&buffer[wasm_object]).unwrap(),
        };

        Ok(this)
    }

    pub fn read_slice(mut serialized: &'a [u8]) -> io::Result<Self> {
        macro_rules! read_int_le {
            ($reader: expr, $ty:ty) => {{
                let reader = $reader;
                reader
                    .split_first_chunk::<{size_of::<$ty>()}>()
                    .map(move |(&int, rest)| {
                        *reader = rest;
                        <$ty>::from_le_bytes(int)
                    })
                    .ok_or_else(|| {
                        io::Error::from(io::ErrorKind::UnexpectedEof)
                    })
            }};
        }

        let read_n = |reader: &mut &'a [u8], len: usize| {
            let Some((read, rest)) = reader.split_at_checked(len) else {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof))
            };
            *reader = rest;
            Ok(read)
        };

        let read_u16_bounded = |reader: &mut &'a [u8]| {
            let len = read_int_le!(&mut *reader, u16)?;
            read_n(reader, usize::from(len))
        };

        let name = str::from_utf8(read_u16_bounded(&mut serialized)?)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

        let start = read_int_le!(&mut serialized, u64)?;
        let length = read_int_le!(&mut serialized, u32)?;
        let output_length = read_int_le!(&mut serialized, u32)?;

        if start.checked_add(u64::from(length)).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                RequestCreationError::OutOfRange
            ))
        }

        let read_u32_bounded = |reader: &mut &'a [u8]| {
            let len = read_int_le!(&mut *reader, u32)?;
            let Ok(len) = usize::try_from(len) else {
                return Err(io::Error::from(io::ErrorKind::OutOfMemory))
            };

            read_n(reader, len)
        };

        let input = read_u32_bounded(&mut serialized)?;
        let wasm_object = read_u32_bounded(&mut serialized)?;


        Ok(Self {
            name: LimitedStr::new(name).unwrap(),
            start,
            length,
            output_length,
            input: LimitedSlice::new(input).unwrap(),
            wasm_object: LimitedSlice::new(wasm_object).unwrap(),
        })
    }

    pub fn write<W: ?Sized + Write>(&self, writer: &mut W) -> io::Result<()> {
        let &Self {
            name,
            start,
            length,
            output_length,
            input,
            wasm_object
        } = self;
        writer.write_u16_le(name.len())?;
        writer.write_all(name.as_str().as_bytes())?;
        writer.write_u64_le(start)?;
        writer.write_u32_le(length)?;
        writer.write_u32_le(output_length)?;
        writer.write_u32_le(input.len())?;
        writer.write_all(input.as_slice())?;
        writer.write_u32_le(wasm_object.len())?;
        writer.write_all(wasm_object.as_slice())?;
        writer.flush()
    }
}


#[derive(Copy, Clone)]
pub struct SystemInfo<'a> {
    cpu_name: LimitedStr<'a, u8>,
    cpu_vendor_id: LimitedStr<'a, u8>,
    cpu_freq: Option<NonZero<u64>>,
    total_memory: Option<NonZero<u64>>,
    cpu_count: Option<NonZero<u32>>,
}

impl<'a> SystemInfo<'a> {
    pub fn current_system(buffer: &'a mut Vec<u8>) -> Self {
        buffer.clear();

        let sys_info = sysinfo::System::new_with_specifics(
            sysinfo::RefreshKind::nothing()
                .with_cpu(sysinfo::CpuRefreshKind::everything())
                .with_memory(sysinfo::MemoryRefreshKind::nothing().with_ram())
        );

        let cpus = sys_info.cpus();
        let name = cpus.first().map(|first| {
            let name = LimitedStr::<u8>::truncated(first.name());
            let vendor = LimitedStr::<u8>::truncated(first.vendor_id());
            let frequency = first.frequency();

            (name, vendor, frequency)
        });

        let (name, vendor_id, frequency) = name
            .map_or_else(
                || {
                    let unknown = LimitedStr::new("<unknown>").unwrap();

                    (unknown, unknown, 0)
                },
                |(name, vendor_id, frequency)| {
                    let name = name.as_str();
                    let vendor_id = vendor_id.as_str();
                    buffer.extend_from_slice(name.as_bytes());
                    let vendor_start_index = match *name == *vendor_id {
                        true => 0,
                        false => {
                            let start = buffer.len();
                            buffer.extend_from_slice(vendor_id.as_bytes());
                            start
                        }
                    };

                    unsafe {
                        let name = str::from_utf8_unchecked(&buffer[..name.len()]);
                        let vendor_id = str::from_utf8_unchecked(&buffer[vendor_start_index..vendor_start_index + vendor_id.len()]);
                        (
                            LimitedStr::new(name).unwrap_unchecked(),
                            LimitedStr::new(vendor_id).unwrap_unchecked(),
                            frequency
                        )
                    }
                }
            );

        let total_memory = sys_info.total_memory();

        Self {
            cpu_name: name,
            cpu_vendor_id: vendor_id,
            cpu_count: NonZero::new(u32::try_from(cpus.len()).unwrap_or(u32::MAX)),
            total_memory: NonZero::new(total_memory),
            cpu_freq: NonZero::new(frequency),
        }
    }

    pub fn cpu_name(&self) -> &'a str {
        self.cpu_name.as_str()
    }

    pub fn cpu_vendor_id(&self) -> &'a str {
        self.cpu_vendor_id.as_str()
    }

    pub fn cpu_freq(&self) -> Option<NonZero<u64>> {
        self.cpu_freq
    }

    pub fn total_memory(&self) -> Option<NonZero<u64>> {
        self.total_memory
    }

    pub fn cpu_count(&self) -> Option<NonZero<u32>> {
        self.cpu_count
    }

    pub async fn read_async<R: ?Sized + AsyncBufRead + Unpin>(buffer: &'a mut Vec<u8>, reader: &mut R) -> io::Result<Self> {
        buffer.clear();

        let read_len = async |reader: &mut R| {
            let mut len = 0_u8;
            reader.read_exact(slice::from_mut(&mut len)).await.map(|_| len)
        };

        let mut read_str = async || {
            let len = usize::from(read_len(reader).await?);
            read_n_async(buffer, reader, len).await
        };

        let cpu_name = read_str().await?;
        let cpu_vendor_id = read_str().await?;

        let mut read_u64 = async || reader.read_u64_le().await.map(NonZero::new);
        let cpu_freq = read_u64().await?;
        let total_memory = read_u64().await?;
        let cpu_count = reader.read_u32_le().await.map(NonZero::new)?;

        let to_str = |range| {
            let str = str::from_utf8(&buffer[range]).map_err(|err| io::Error::new(
                io::ErrorKind::InvalidData,
                err
            ));

            str.map(|str| LimitedStr::new(str).unwrap())
        };

        Ok(Self {
            cpu_name: to_str(cpu_name)?,
            cpu_vendor_id: to_str(cpu_vendor_id)?,
            cpu_freq,
            total_memory,
            cpu_count,
        })
    }

    pub fn write<W: ?Sized + Write>(&self, writer: &mut W) -> io::Result<()> {
        let &Self {
            cpu_name,
            cpu_vendor_id,
            cpu_freq,
            total_memory,
            cpu_count
        } = self;
        writer.write_all(&[cpu_name.len()])?;
        writer.write_all(cpu_name.as_str().as_bytes())?;
        writer.write_all(&[cpu_vendor_id.len()])?;
        writer.write_all(cpu_vendor_id.as_str().as_bytes())?;
        writer.write_u64_le(cpu_freq.map_or(0, NonZero::get))?;
        writer.write_u64_le(total_memory.map_or(0, NonZero::get))?;
        writer.write_u32_le(cpu_count.map_or(0, NonZero::get))?;
        writer.flush()
    }
}