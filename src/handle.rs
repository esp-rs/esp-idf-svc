pub trait RawHandle {
    type Handle;

    unsafe fn handle(&self) -> Handle;
}
