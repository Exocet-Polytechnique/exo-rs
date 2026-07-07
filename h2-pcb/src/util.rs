// WARNING: no checks performed by this function
pub fn to_fixed16(value: f32, n: usize) -> u16 {
    (value * ((2 ^ n) as f32)) as u16
}