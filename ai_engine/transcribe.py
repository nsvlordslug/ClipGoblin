#!/usr/bin/env python3
"""
Speech-to-text transcription using faster-whisper.
Extracts word-level timestamps for auto-captions and keyword detection.

Usage:
    python transcribe.py <video_path> [--model small] [--output transcript.json]

Output JSON format:
{
    "segments": [
        {
            "start": 0.0,
            "end": 2.5,
            "text": "No way, did you see that?",
            "words": [
                {"word": "No", "start": 0.0, "end": 0.3},
                {"word": "way,", "start": 0.3, "end": 0.6},
                ...
            ]
        }
    ],
    "full_text": "No way, did you see that? ...",
    "language": "en",
    "keywords_found": [
        {"keyword": "no way", "timestamp": 0.0, "context": "No way, did you see that?"}
    ]
}
"""

import sys
import json
import argparse
import os
import re
import time

# Viral trigger phrases — detection of these boosts clip scores
VIRAL_KEYWORDS = [
    "no way", "what the", "oh my god", "holy shit", "holy crap",
    "are you kidding", "let's go", "lets go", "get out", "wait what",
    "dude", "bro", "bruh", "insane", "crazy", "unbelievable",
    "oh no", "oh god", "what the hell", "what the fuck", "wtf",
    "clutch", "poggers", "pog", "gg", "ez", "destroyed",
    "i can't", "i cant", "stop", "shut up", "run", "go go go",
    "help", "why", "how", "impossible", "hacker", "cheater",
    "rage", "rage quit", "i'm done", "im done", "that's it",
    "watch this", "look at this", "did you see", "clip it",
    "clip that", "highlight", "oh snap", "yooo", "yoo",
    "noooo", "nooo", "yesss", "yes sir", "baby",
    "w", "massive", "huge", "epic", "legendary",
]


def find_keywords(segments):
    """Find viral keywords in transcribed segments."""
    found = []
    for seg in segments:
        text_lower = seg["text"].lower()
        for kw in VIRAL_KEYWORDS:
            if kw in text_lower:
                found.append({
                    "keyword": kw,
                    "timestamp": seg["start"],
                    "end_timestamp": seg["end"],
                    "context": seg["text"].strip(),
                })
    return found


def transcribe(video_path, model_size="small", output_path=None):
    """Run faster-whisper transcription on a video file."""
    try:
        from faster_whisper import WhisperModel
    except ImportError:
        print(json.dumps({"error": "faster-whisper not installed"}))
        sys.exit(1)

    if not os.path.exists(video_path):
        print(json.dumps({"error": f"File not found: {video_path}"}))
        sys.exit(1)

    # Use CPU by default — reliable without CUDA/cuBLAS.
    # Set CLIPGOBLIN_DEVICE=cuda to force GPU if CUDA is properly installed.
    device = os.environ.get("CLIPGOBLIN_DEVICE", "cpu")
    compute = "float16" if device == "cuda" else "int8"
    try:
        model = WhisperModel(model_size, device=device, compute_type=compute)
    except Exception as e:
        if device != "cpu":
            # GPU failed — fall back to CPU
            try:
                model = WhisperModel(model_size, device="cpu", compute_type="int8")
            except Exception as e2:
                print(json.dumps({"error": f"Failed to load model: {str(e2)}"}))
                sys.exit(1)
        else:
            print(json.dumps({"error": f"Failed to load model: {str(e)}"}))
            sys.exit(1)

    # Helper: iterate the segment generator with periodic heartbeat output.
    # faster-whisper yields segments lazily, so a long VOD can block for
    # minutes between yields.  We emit JSON heartbeats to stderr every
    # HEARTBEAT_INTERVAL seconds so the Rust parent process knows we're alive.
    HEARTBEAT_INTERVAL = 15  # seconds

    def collect_segments_with_heartbeat(gen, duration_hint):
        """Iterate segment generator, emitting heartbeat JSON to stderr."""
        collected = []
        last_beat = time.monotonic()
        for seg in gen:
            collected.append(seg)
            now = time.monotonic()
            if now - last_beat >= HEARTBEAT_INTERVAL:
                last_beat = now
                pct = 0
                if duration_hint and duration_hint > 0 and seg.end > 0:
                    pct = min(99, int(seg.end / duration_hint * 100))
                print(json.dumps({
                    "heartbeat": True,
                    "segments_so_far": len(collected),
                    "last_timestamp": round(seg.end, 1),
                    "approx_pct": pct,
                }), file=sys.stderr, flush=True)
        return collected

    # Transcribe with word timestamps.
    # Wrap in try/except — CUDA can load fine but crash during inference
    # (OOM, cuBLAS errors, driver issues).  If that happens, rebuild the
    # model on CPU and retry.
    try:
        segments_gen, info = model.transcribe(
            video_path,
            beam_size=5,
            word_timestamps=True,
            vad_filter=True,  # Skip silence
            vad_parameters=dict(
                min_silence_duration_ms=500,
                speech_pad_ms=200,
            ),
        )
        duration_hint = info.duration if info else 0
        raw_segments = collect_segments_with_heartbeat(segments_gen, duration_hint)
    except Exception as e:
        if device == "cpu":
            # Already on CPU — nothing to fall back to
            print(json.dumps({"error": f"Transcription failed: {str(e)}"}))
            sys.exit(1)

        # CUDA failed during transcription — fall back to CPU
        print(json.dumps({
            "progress_message": f"CUDA unavailable ({type(e).__name__}), using CPU transcription (slower)"
        }), flush=True)

        try:
            model = WhisperModel(model_size, device="cpu", compute_type="int8")
            segments_gen, info = model.transcribe(
                video_path,
                beam_size=5,
                word_timestamps=True,
                vad_filter=True,
                vad_parameters=dict(
                    min_silence_duration_ms=500,
                    speech_pad_ms=200,
                ),
            )
            duration_hint = info.duration if info else 0
            raw_segments = collect_segments_with_heartbeat(segments_gen, duration_hint)
        except Exception as e2:
            print(json.dumps({"error": f"Transcription failed on both CUDA and CPU: {str(e2)}"}))
            sys.exit(1)

    segments = []
    full_text_parts = []

    for segment in raw_segments:
        words = []
        if segment.words:
            for w in segment.words:
                words.append({
                    "word": w.word.strip(),
                    "start": round(w.start, 2),
                    "end": round(w.end, 2),
                })

        seg_data = {
            "start": round(segment.start, 2),
            "end": round(segment.end, 2),
            "text": segment.text.strip(),
            "words": words,
        }
        segments.append(seg_data)
        full_text_parts.append(segment.text.strip())

    full_text = " ".join(full_text_parts)
    keywords = find_keywords(segments)

    result = {
        "segments": segments,
        "full_text": full_text,
        "language": info.language if info else "unknown",
        "language_probability": round(info.language_probability, 2) if info else 0,
        "duration": round(info.duration, 2) if info else 0,
        "keywords_found": keywords,
    }

    if output_path:
        with open(output_path, "w", encoding="utf-8") as f:
            json.dump(result, f, ensure_ascii=False, indent=2)
        # Also print to stdout for Rust to capture
        print(json.dumps({"status": "ok", "output": output_path, "segments": len(segments), "keywords": len(keywords)}))
    else:
        print(json.dumps(result, ensure_ascii=False))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Transcribe video with faster-whisper")
    parser.add_argument("video_path", help="Path to video file")
    parser.add_argument("--model", default="small", help="Whisper model size (tiny/base/small/medium/large-v3)")
    parser.add_argument("--device", default=None, help="Device: cpu or cuda (overrides CLIPGOBLIN_DEVICE env var)")
    parser.add_argument("--output", default=None, help="Output JSON file path")
    args = parser.parse_args()

    # CLI --device overrides env var
    if args.device:
        os.environ["CLIPGOBLIN_DEVICE"] = args.device

    transcribe(args.video_path, args.model, args.output)
