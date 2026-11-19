pub mod scalar;

pub type Adler32Imp = fn(u16, u16, &[u8]) -> (u16, u16);

pub fn get_imp() -> Adler32Imp {
  scalar::update
}
