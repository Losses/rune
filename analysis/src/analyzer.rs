use crate::base_analyzer::BaseAnalyzer;

pub trait SubAnalyzer: Sync + Send {
    fn process_audio_chunk(&mut self, base_analyzer: &mut BaseAnalyzer, chunk: &[f32], force: bool);
}
