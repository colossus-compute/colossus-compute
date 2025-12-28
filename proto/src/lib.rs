use std::io;
use std::ops::Range;
use tokio::io::{AsyncBufRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub struct Request<'a> {
    name: &'a str,
    start: u64,
    length: u32,
    output_length: u32,
    input: &'a [u8],
    wasm_object: &'a [u8]
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
        if u16::try_from(name.len()).is_err() {
            return Err(RequestCreationError::NameTooLong)
        }

        if start.checked_add(u64::from(length)).is_none() {
            return Err(RequestCreationError::OutOfRange)
        }

        if u32::try_from(input.len()).is_err() {
            return Err(RequestCreationError::InputTooBig)
        }

        if u32::try_from(wasm_object.len()).is_err() {
            return Err(RequestCreationError::WasmObjectTooBig)
        }

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
        self.name
    }

    pub fn name_len(&self) -> u16 {
        unsafe { u16::try_from(self.name.len()).unwrap_unchecked() }
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
        self.input
    }

    pub fn input_len(&self) -> u32 {
        unsafe { u32::try_from(self.input.len()).unwrap_unchecked() }
    }

    pub fn wasm_object_len(&self) -> u32 {
        unsafe { u32::try_from(self.wasm_object.len()).unwrap_unchecked() }
    }

    pub fn wasm_object(&self) -> &'a [u8] {
        self.wasm_object
    }

    pub fn output_length(&self) -> u32 {
        self.output_length
    }
}


impl<'a> Request<'a> {
    // TODO: use buffer for bufread
    pub async fn read<R: AsyncBufRead + Unpin>(buffer: &'a mut Vec<u8>, reader: &mut R) -> io::Result<(Self, usize)> {
        buffer.clear();

        let mut read_n = async |reader: &mut R, len: usize| {
            let Ok(len_64) = u64::try_from(len) else {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof))
            };

            buffer.try_reserve(len)?;

            let start = buffer.len();
            let n = reader
                .take(len_64)
                .read_to_end(buffer)
                .await?;

            if n != len {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof))
            }
            let end = start + len;
            assert_eq!(end, buffer.len());

            Ok(start..end)
        };
        let mut read_u16_bounded = async |reader: &mut R| {
            let len = reader.read_u16_le().await?;
            read_n(reader, usize::from(len)).await
        };

        let name = read_u16_bounded(reader).await?;

        let start = reader.read_u64_le().await?;
        let length = reader.read_u32_le().await?;
        let output_length = reader.read_u32_le().await?;

        if start.checked_add(u64::from(length)).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                RequestCreationError::OutOfRange
            ))
        }

        let mut read_u32_bounded = async |reader: &mut R|  {
            let len = reader.read_u32_le().await?;
            let Ok(len) = usize::try_from(len) else {
                return Err(io::Error::from(io::ErrorKind::OutOfMemory))
            };

            read_n(reader, len).await
        };

        let input = read_u32_bounded(reader).await?;
        let wasm_object = read_u32_bounded(reader).await?;

        let name = str::from_utf8(&buffer[name])
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;

        let this = Self {
            name,
            start,
            length,
            output_length,
            input: &buffer[input],
            wasm_object: &buffer[wasm_object],
        };

        Ok((this, buffer.len()))
    }

    pub async fn write<W: AsyncWrite + Unpin>(&self, writer: &mut W) -> io::Result<()> {
        let &Self {
            name,
            start,
            length,
            output_length,
            input,
            wasm_object
        } = self;
        writer.write_u16_le(self.name_len()).await?;
        writer.write_all(name.as_bytes()).await?;
        writer.write_u64_le(start).await?;
        writer.write_u32_le(length).await?;
        writer.write_u32_le(output_length).await?;
        writer.write_u32(self.input_len()).await?;
        writer.write_all(input).await?;
        writer.write_u32(self.wasm_object_len()).await?;
        writer.write_all(wasm_object).await?;
        writer.flush().await
    }
}