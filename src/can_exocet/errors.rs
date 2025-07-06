#[derive(Debug)]
pub enum CanError {
    InvalidModule,
    FrameError,
    DroppedFrame
}