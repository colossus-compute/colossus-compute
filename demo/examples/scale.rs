
fn main() {
    let mut array: [f32; 4] = [1.0, 2.0, 3.0, 4.0];

    let scale_module = include_bytes!("../jobs/scale.wat");
    demo::map_slice(
        "scale input",
        &mut array,
        scale_module
    ).unwrap();

    println!("{array:?}")
}