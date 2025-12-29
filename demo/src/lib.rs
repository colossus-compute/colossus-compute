use std::borrow::Cow;
use std::io;
use std::io::Read;

pub fn run_raw(
    name: &str,
    start: u64,
    length: u32,
    output_len: u32,
    input: &[u8],
    wasm: &[u8]
) -> Result<impl Read + use<>, ureq::Error> {
    let request = proto::Request::new(
        name,
        start,
        length,
        output_len,
        input,
        wasm
    );

    let request = request
        .map_err(|err| ureq::Error::Other(Box::new(err)))?;

    let mut bytes = vec![];
    request.write(&mut bytes)?;

    eprintln!("sending request...");
    let request = ureq::post("http://localhost:42371/submit")
        .send(bytes)?;

    eprintln!("sent request!");

    Ok(request.into_body().into_reader())
}

pub trait ToLeBytes: Sized + Copy + bytemuck::Pod {
    fn make_le_bytes(slice: &mut [Self]);
    fn cast_le_bytes(slice: &[Self]) -> Cow<'_, [u8]>;
}

impl ToLeBytes for u8 {
    fn make_le_bytes(slice: &mut [Self]) {
        let _ = slice;
    }

    fn cast_le_bytes(slice: &[Self]) -> Cow<'_, [u8]> {
        Cow::Borrowed(slice)
    }
}

impl ToLeBytes for i8 {
    fn make_le_bytes(slice: &mut [Self]) {
        let _ = slice;
    }

    fn cast_le_bytes(slice: &[Self]) -> Cow<'_, [u8]> {
        Cow::Borrowed(bytemuck::must_cast_slice(slice))
    }
}


macro_rules! to_le_map {
    ($ty: ty; Map($func: expr)) => {
        impl ToLeBytes for $ty {
            fn make_le_bytes(slice: &mut [Self]) {
                if !cfg!(target_endian = "little") {
                    for item in slice {
                        let tmp: $ty = *item;
                        *item = ($func)(tmp);
                    }
                }
            }

            fn cast_le_bytes(slice: &[Self]) -> Cow<'_, [u8]> {
                match cfg!(target_endian = "little") {
                    true => Cow::Borrowed(bytemuck::must_cast_slice(slice)),
                    false => {
                        let mut this = slice.to_vec();
                        Self::make_le_bytes(&mut this);
                        Cow::Owned(bytemuck::cast_vec(this))
                    }
                }
            }
        }
    };
}

macro_rules! to_le_int {
    ($($int: ty)*) => {
        $(to_le_map! { $int ; Map(<$int>::to_le) })*
    };
}

macro_rules! to_le_float {
    ($($float: ty)*) => {
        $(to_le_map! {
            $float ;
            Map(|float| {
                <$float>::from_bits(<$float>::to_bits(float).to_le())
            })
        })*
    };
}

to_le_int! {
    u16 u32 u64 u128 usize
    i16 i32 i64 i128 isize
}

to_le_float! {
    f32 f64
}

pub fn map_slice<T: ToLeBytes>(
    name: &str,
    slice: &mut [T],
    wasm: &[u8],
) -> Result<(), ureq::Error> {
    let length = u32::try_from(slice.len()).unwrap();
    let input = &*T::cast_le_bytes(slice);
    let input_len = size_of_val::<[u8]>(input);
    let output_len = u32::try_from(input_len).unwrap();

    let mut reader = run_raw(
        name,
        0,
        length,
        output_len,
        input,
        wasm
    )?;

    let output = bytemuck::must_cast_slice_mut(slice);
    reader.read_exact(output)?;
    if reader.read(&mut [0])? != 0 {
        return Err(ureq::Error::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            "output was too long"
        )));
    }
    T::make_le_bytes(slice);

    Ok(())
}
