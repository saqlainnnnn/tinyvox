#[derive(Debug, Clone)]
pub struct AudioBuffer {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

pub trait AudioRecorder {
    type Error;

    fn start(&mut self) -> Result<(), Self::Error>;

    fn stop(&mut self) -> Result<AudioBuffer, Self::Error>;
}
