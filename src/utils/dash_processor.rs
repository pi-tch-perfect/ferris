use std::process::Command;
use tracing::{debug, error};

use crate::globals;

#[derive(Debug)]
pub enum ProcessingMode {
    Copy,
    PitchShift(Vec<i32>),
}

pub struct DashProcessor {
    segment_duration: u32,
}

impl DashProcessor {
    pub fn new(segment_duration: u32) -> Self {
        DashProcessor { segment_duration }
    }

    fn build_filter_complex(&self, mode: &ProcessingMode) -> Option<String> {
        match mode {
            ProcessingMode::Copy => {
                Some("[0:a]loudnorm=I=-16:TP=-1.5:LRA=11[normalized]".to_string())
            }
            ProcessingMode::PitchShift(shifts) => {
                let num_streams = shifts.len();
                let mut filter = format!("[0:a]asplit={}", num_streams);

                // Create split points
                for i in 0..num_streams {
                    filter.push_str(&format!("[a{}]", i));
                }
                filter.push(';');

                // Process each stream with pitch shift and normalization
                for (i, semitones) in shifts.iter().enumerate() {
                    let rate_multiplier = 2f64.powf(*semitones as f64 / 12.0);
                    filter.push_str(&format!(
                        " [a{}]rubberband=pitch={},loudnorm=I=-16:TP=-1.5:LRA=11[p{}];",
                        i, rate_multiplier, i
                    ));
                }

                filter.pop(); // Remove the last semicolon
                Some(filter)
            }
        }
    }

    fn build_adaptation_sets(&self, mode: &ProcessingMode) -> String {
        match mode {
            ProcessingMode::Copy => "id=0,streams=0 id=1,streams=1".to_string(),
            ProcessingMode::PitchShift(shifts) => {
                let mut adaptation_sets = String::from("id=0,streams=0 ");
                for (i, _) in shifts.iter().enumerate() {
                    adaptation_sets.push_str(&format!("id={},streams={} ", i + 1, i + 1));
                }
                adaptation_sets.trim().to_string()
            }
        }
    }

    fn build_stream_mappings(&self, mode: &ProcessingMode) -> Vec<String> {
        let mut mappings = vec!["-map".to_string(), "0:v".to_string()];

        match mode {
            ProcessingMode::Copy => {
                mappings.extend(vec!["-map".to_string(), "[normalized]".to_string()]);
            }
            ProcessingMode::PitchShift(shifts) => {
                for i in 0..shifts.len() {
                    mappings.push("-map".to_string());
                    mappings.push(format!("[p{}]", i));
                }
            }
        }

        mappings
    }

    fn build_audio_encodings(&self, mode: &ProcessingMode) -> Vec<String> {
        let mut encodings = Vec::new();

        match mode {
            ProcessingMode::Copy => {
                encodings.extend(vec![
                    "-c:a".to_string(),
                    "aac".to_string(),
                    "-b:a".to_string(),
                    "128k".to_string(),
                ]);
            }
            ProcessingMode::PitchShift(shifts) => {
                for i in 0..shifts.len() {
                    encodings.push(format!("-c:a:{}", i));
                    encodings.push("aac".to_string());
                    encodings.push(format!("-b:a:{}", i));
                    encodings.push("128k".to_string());
                }
            }
        }

        encodings
    }

    pub fn execute(
        &self,
        input_file: &str,
        output_file: &str,
        mode: &ProcessingMode,
    ) -> std::io::Result<()> {
        let ffmpeg_path = globals::get_binary_path("ffmpeg");
        debug!("Using FFmpeg from path: {}", ffmpeg_path.display());

        let mut command = Command::new(ffmpeg_path);
        command.arg("-i").arg(input_file).arg("-c:v").arg("copy");

        // Add filter complex if needed
        if let Some(filter_complex) = self.build_filter_complex(mode) {
            command.arg("-filter_complex").arg(filter_complex);
        }

        command
            .args(self.build_stream_mappings(mode))
            .args(self.build_audio_encodings(mode))
            .arg("-f")
            .arg("dash")
            .arg("-adaptation_sets")
            .arg(self.build_adaptation_sets(mode))
            .arg("-seg_duration")
            .arg(self.segment_duration.to_string())
            .arg(output_file);

        debug!("ffmpeg command: {:?}", command);

        let output = command.output()?;
        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            error!("FFmpeg error: {}", error);
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "FFmpeg command failed",
            ));
        }
        Ok(())
    }
}
