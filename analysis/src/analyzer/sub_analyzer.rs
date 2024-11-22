use crate::analyzer::core_analyzer::Analyzer;

pub trait SubAnalyzer: Sync + Send {
    fn process_audio_chunk(&mut self, base_analyzer: &mut Analyzer, chunk: &[f32], force: bool);
}
