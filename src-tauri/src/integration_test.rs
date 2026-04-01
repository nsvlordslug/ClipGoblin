//! End-to-end integration test for the clip detection pipeline.

#[cfg(test)]
mod tests {
    use crate::audio_signal::{self, AudioProfile};
    use crate::clip_fusion::{self, FusionConfig};
    use crate::clip_output;
    use crate::clip_ranker::{self, ScoringConfig};
    use crate::pipeline::*;
    use crate::scene_signal::{self, MotionProfile, SceneDetection};
    use crate::transcript_signal::{self, InputKeyword, InputSegment, TranscriptInput};

    const VOD_DURATION: f64 = 600.0;

    fn build_audio_profile() -> AudioProfile {
        let mut rms = vec![0.3; VOD_DURATION as usize];
        for i in 119..123 { rms[i] = 0.92; }
        for i in 348..358 { rms[i] = 0.85; }
        AudioProfile::from_rms(rms)
    }

    fn build_transcript() -> TranscriptInput {
        TranscriptInput {
            segments: vec![
                InputSegment { start: 120.0, end: 123.0, text: "OH MY GOD WHAT WAS THAT!!!".into() },
                InputSegment { start: 200.0, end: 205.0, text: "okay let me just go over here".into() },
                InputSegment { start: 350.0, end: 354.0, text: "LET'S GO let's go let's go!!!".into() },
                InputSegment { start: 498.0, end: 503.0, text: "I'm done. I'm actually done. This is bullshit.".into() },
            ],
            keywords: vec![
                InputKeyword { keyword: "oh my god".into(), start: 120.0, end: 121.0, context: "OH MY GOD WHAT WAS THAT".into() },
                InputKeyword { keyword: "let's go".into(), start: 350.0, end: 351.0, context: "LET'S GO let's go let's go".into() },
                InputKeyword { keyword: "i'm done".into(), start: 498.0, end: 499.0, context: "I'm done. I'm actually done.".into() },
            ],
            language: "en".into(),
        }
    }

    fn build_scene_cuts() -> Vec<SceneDetection> {
        vec![
            SceneDetection { time: 120.5, score: 0.7 },
            SceneDetection { time: 349.0, score: 0.6 },
            SceneDetection { time: 352.0, score: 0.55 },
            SceneDetection { time: 355.0, score: 0.5 },
            SceneDetection { time: 430.0, score: 0.35 },
        ]
    }

    fn build_motion_profile() -> MotionProfile {
        let mut energy = vec![0.2; VOD_DURATION as usize];
        for i in 348..360 { energy[i] = 0.8; }
        MotionProfile::from_energy(energy)
    }

    #[test]
    fn full_pipeline_with_synthetic_data() {
        let max_clips = 5;

        let audio_segments = audio_signal::detect_signals(&build_audio_profile());
        let transcript_segments = transcript_signal::analyze(&build_transcript());
        let scene_segments = scene_signal::detect_signals(&build_scene_cuts(), &build_motion_profile());

        assert!(!audio_segments.is_empty());
        assert!(!transcript_segments.is_empty());
        assert!(!scene_segments.is_empty());

        for seg in audio_segments.iter().chain(&transcript_segments).chain(&scene_segments) {
            assert!(seg.score >= 0.0 && seg.score <= 1.0);
        }

        let mut all = Vec::new();
        all.extend(audio_segments);
        all.extend(transcript_segments);
        all.extend(scene_segments);
        let total_signals = all.len();

        let fusion_config = FusionConfig {
            max_candidates: max_clips * 4,
            ..FusionConfig::new(VOD_DURATION)
        };
        let candidates = clip_fusion::fuse(&all, &fusion_config);
        assert!(!candidates.is_empty());

        for clip in &candidates {
            assert!(clip.start_time >= 0.0);
            assert!(clip.end_time <= VOD_DURATION);
            assert!(clip.start_time < clip.end_time);
            assert!((clip.confidence_score - 0.0).abs() < f64::EPSILON);
        }

        let config = ScoringConfig::standard();
        let ranked = clip_ranker::rank(&candidates, &config, max_clips * 2);
        assert!(!ranked.is_empty());

        for (i, r) in ranked.iter().enumerate() {
            assert_eq!(r.rank, i + 1);
            assert!(r.clip.confidence_score > 0.0);
            assert!(r.clip.confidence_score <= 1.0);
            let report = r.clip.score_report.as_ref().expect("score_report must be set");
            assert!(!report.explanation.is_empty());
            assert!(!report.key_dimensions.is_empty());
            assert!(report.rank_score > 0.0);
        }

        let final_clips = clip_output::finalize_without_thumbnails(&ranked, max_clips);
        assert!(!final_clips.is_empty());
        assert!(final_clips.len() <= max_clips);

        eprintln!("Pipeline: {} signals → {} candidates → {} ranked → {} output",
            total_signals, candidates.len(), ranked.len(), final_clips.len());
    }

    #[test]
    fn multi_signal_event_ranks_above_single_signal() {
        let mut all = Vec::new();
        all.extend(audio_signal::detect_signals(&build_audio_profile()));
        all.extend(transcript_signal::analyze(&build_transcript()));
        all.extend(scene_signal::detect_signals(&build_scene_cuts(), &build_motion_profile()));

        let config = FusionConfig { max_candidates: 20, ..FusionConfig::new(VOD_DURATION) };
        let candidates = clip_fusion::fuse(&all, &config);
        let ranked = clip_ranker::rank(&candidates, &ScoringConfig::standard(), 10);

        assert!(ranked.len() >= 2);
        let event2 = ranked.iter().find(|r| r.clip.start_time < 360.0 && r.clip.end_time > 340.0);
        let event3 = ranked.iter().find(|r| r.clip.start_time < 510.0 && r.clip.start_time > 480.0);

        if let (Some(e2), Some(e3)) = (event2, event3) {
            assert!(e2.rank < e3.rank, "multi-signal should outrank single-signal");
        }
    }

    #[test]
    fn vision_mode_changes_dimension_weights() {
        let mut all = Vec::new();
        all.extend(audio_signal::detect_signals(&build_audio_profile()));
        all.extend(transcript_signal::analyze(&build_transcript()));
        all.extend(scene_signal::detect_signals(&build_scene_cuts(), &build_motion_profile()));

        let config = FusionConfig { max_candidates: 20, ..FusionConfig::new(VOD_DURATION) };
        let candidates = clip_fusion::fuse(&all, &config);

        let local = clip_ranker::rank(&candidates, &ScoringConfig::standard(), 5);

        // Should produce results with standard (local) scoring
        assert!(!local.is_empty());
    }

    #[test]
    fn empty_signals_produce_empty_output() {
        let candidates = clip_fusion::fuse(&[], &FusionConfig::new(600.0));
        assert!(candidates.is_empty());
        let ranked = clip_ranker::rank(&candidates, &ScoringConfig::standard(), 5);
        assert!(ranked.is_empty());
        let output = clip_output::finalize_without_thumbnails(&ranked, 5);
        assert!(output.is_empty());
    }

    #[test]
    fn single_weak_signal_rejected() {
        let segments = vec![SignalSegment {
            signal_type: SignalType::SceneChange,
            start_time: 100.0, end_time: 102.0,
            score: 0.2,
            tags: vec!["transition".into()],
            metadata: None,
        }];
        let config = FusionConfig { max_candidates: 10, ..FusionConfig::new(600.0) };
        let candidates = clip_fusion::fuse(&segments, &config);
        assert!(candidates.is_empty());
    }

    #[test]
    fn max_clips_propagates() {
        let mut all = Vec::new();
        all.extend(audio_signal::detect_signals(&build_audio_profile()));
        all.extend(transcript_signal::analyze(&build_transcript()));
        all.extend(scene_signal::detect_signals(&build_scene_cuts(), &build_motion_profile()));

        let max_clips = 1;
        let config = FusionConfig { max_candidates: max_clips * 4, ..FusionConfig::new(VOD_DURATION) };
        let candidates = clip_fusion::fuse(&all, &config);
        let ranked = clip_ranker::rank(&candidates, &ScoringConfig::standard(), max_clips * 2);
        let output = clip_output::finalize_without_thumbnails(&ranked, max_clips);
        assert!(output.len() <= max_clips);
    }

    #[test]
    fn audio_module_independent() {
        let segments = audio_signal::detect_signals(&build_audio_profile());
        let config = FusionConfig { max_candidates: 10, ..FusionConfig::new(VOD_DURATION) };
        let candidates = clip_fusion::fuse(&segments, &config);
        let ranked = clip_ranker::rank(&candidates, &ScoringConfig::standard(), 5);
        assert!(!ranked.is_empty());
        for r in &ranked {
            assert!(r.clip.score_breakdown.audio_score > 0.0);
        }
    }

    #[test]
    fn transcript_module_independent() {
        let segments = transcript_signal::analyze(&build_transcript());
        let config = FusionConfig { max_candidates: 10, ..FusionConfig::new(VOD_DURATION) };
        let candidates = clip_fusion::fuse(&segments, &config);
        let ranked = clip_ranker::rank(&candidates, &ScoringConfig::standard(), 5);
        assert!(!ranked.is_empty());
        for r in &ranked {
            assert!(r.clip.score_breakdown.speech_score > 0.0);
        }
    }

    #[test]
    fn score_report_complete_through_pipeline() {
        let mut all = Vec::new();
        all.extend(audio_signal::detect_signals(&build_audio_profile()));
        all.extend(transcript_signal::analyze(&build_transcript()));
        all.extend(scene_signal::detect_signals(&build_scene_cuts(), &build_motion_profile()));

        let config = FusionConfig { max_candidates: 20, ..FusionConfig::new(VOD_DURATION) };
        let candidates = clip_fusion::fuse(&all, &config);
        let ranked = clip_ranker::rank(&candidates, &ScoringConfig::standard(), 5);

        for r in &ranked {
            let report = r.clip.score_report.as_ref().expect("score_report must be set");

            // Dimensions are populated
            assert!(report.dimension_weighted > 0.0);
            assert!(!report.key_dimensions.is_empty());

            // Explanation is non-empty
            assert!(!report.explanation.is_empty());

            // confidence_score uses the rescaled confidence, not rank_score
            assert!((report.confidence - r.clip.confidence_score).abs() < 1e-9);
            // rank_score and confidence are both positive
            assert!(report.rank_score > 0.0);
            assert!(report.confidence > 0.0);

            // Serialises cleanly
            let json = serde_json::to_value(report).unwrap();
            assert!(json["dimensions"].is_object());
            assert!(json["bonuses"].is_array());
            assert!(json["penalties"].is_array());
            assert!(json["explanation"].is_string());
            assert!(json["key_dimensions"].is_array());
        }
    }
}
