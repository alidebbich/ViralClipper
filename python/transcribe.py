import sys
import json
import os
import time

# Ensure UTF-8 output on Windows
sys.stdout.reconfigure(encoding='utf-8')
sys.stderr.reconfigure(encoding='utf-8')

def main():
    if len(sys.argv) < 2:
        print(json.dumps({"error": "Usage: python transcribe.py <video_path>"}))
        sys.exit(1)
    
    video_path = sys.argv[1]
    
    # Check if Parakeet ONNX model exists
    base_dir = os.path.dirname(__file__)
    # Check both python/models and ../models
    candidate_dirs = [
        os.path.join(base_dir, 'models', 'parakeet'),
        os.path.join(os.path.dirname(base_dir), 'models', 'parakeet'),
        os.path.join(base_dir, 'models', 'parakeet', 'sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8'),
        os.path.join(os.path.dirname(base_dir), 'models', 'parakeet', 'sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8')
    ]
    
    parakeet_dir = None
    for d in candidate_dirs:
        if os.path.exists(os.path.join(d, "encoder.int8.onnx")):
            parakeet_dir = d
            break
            
    if not parakeet_dir:
        parakeet_dir = os.path.join(base_dir, 'models', 'parakeet')
        os.makedirs(parakeet_dir, exist_ok=True)
    
    use_parakeet = False
    parakeet_result = None
    
    try:
        import sherpa_onnx
        import soundfile as sf
        import urllib.request
        
        model_url = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2"
        model_file = "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2"
        
        if not os.path.exists(os.path.join(parakeet_dir, "encoder.int8.onnx")):
            nested = os.path.join(parakeet_dir, "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8")
            if os.path.exists(os.path.join(nested, "encoder.int8.onnx")):
                parakeet_dir = nested
            else:
                import tarfile
                print(json.dumps({"info": f"Downloading Parakeet model... ({model_url})"}), file=sys.stderr)
                urllib.request.urlretrieve(model_url, os.path.join(parakeet_dir, model_file))
                with tarfile.open(os.path.join(parakeet_dir, model_file), "r:bz2") as tar:
                    tar.extractall(path=parakeet_dir)
                os.remove(os.path.join(parakeet_dir, model_file))
                if os.path.exists(os.path.join(nested, "encoder.int8.onnx")):
                    parakeet_dir = nested

        # Try loading Parakeet (provider=cpu is safe, fast, and does not crash if CUDA DLLs missing)
        recognizer = sherpa_onnx.OfflineRecognizer.from_transducer(
            encoder=os.path.join(parakeet_dir, "encoder.int8.onnx"),
            decoder=os.path.join(parakeet_dir, "decoder.int8.onnx"),
            joiner=os.path.join(parakeet_dir, "joiner.int8.onnx"),
            tokens=os.path.join(parakeet_dir, "tokens.txt"),
            num_threads=4,
            provider="cpu",
            model_type="nemo_transducer"
        )
        
        audio_data, sample_rate = sf.read(video_path, dtype="float32")
        if len(audio_data.shape) > 1:
            audio_data = audio_data[:, 0]
            
        stream = recognizer.create_stream()
        stream.accept_waveform(sample_rate, audio_data)
        recognizer.decode_stream(stream)
        res = stream.result
        
        # Parse Parakeet BPE tokens into words
        words = []
        current_word = ""
        current_start = 0.0
        
        for token, timestamp in zip(res.tokens, res.timestamps):
            if token.startswith(" "):
                if current_word:
                    words.append({
                        "start": current_start,
                        "end": timestamp,
                        "text": current_word.strip(),
                        "confidence": 1.0
                    })
                current_word = token[1:]
                current_start = timestamp
            else:
                current_word += token
                
        if current_word:
            words.append({
                "start": current_start,
                "end": res.timestamps[-1] if res.timestamps else current_start + 0.5,
                "text": current_word.strip(),
                "confidence": 1.0
            })
            
        # Group into ~5 second segments
        segments = []
        current_segment = []
        seg_start = 0.0
        
        for w in words:
            if not current_segment:
                seg_start = w["start"]
            current_segment.append(w)
            
            if w["end"] - seg_start > 5.0:
                segments.append({
                    "start": seg_start,
                    "end": w["end"],
                    "text": " ".join([x["text"] for x in current_segment]),
                    "words": current_segment
                })
                current_segment = []
                
        if current_segment:
            segments.append({
                "start": seg_start,
                "end": current_segment[-1]["end"],
                "text": " ".join([x["text"] for x in current_segment]),
                "words": current_segment
            })
            
        # If Parakeet found valid words (meaning English audio), use it!
        if len(words) >= 5:
            parakeet_result = {
                "language": "en",
                "duration": len(audio_data) / sample_rate,
                "segments": segments
            }
            use_parakeet = True
        else:
            print(json.dumps({"info": "Parakeet produced minimal tokens (non-English or music), falling back to Whisper..."}), file=sys.stderr)
            
    except Exception as e:
        print(json.dumps({"info": f"Parakeet unavailable, falling back to Whisper: {e}"}), file=sys.stderr)
        
    if use_parakeet and parakeet_result:
        print(json.dumps(parakeet_result, ensure_ascii=False))
        return

    # ---------------------------------------------------------
    # MULTILINGUAL WHISPER FALLBACK PATH (fast, robust, any language)
    # ---------------------------------------------------------
    try:
        from faster_whisper import WhisperModel
    except ImportError:
        print(json.dumps({"error": "faster-whisper not installed."}))
        sys.exit(1)
        
    models_dir = os.path.join(base_dir, 'models')
    
    # Register NVIDIA CUDA & cuDNN DLL directories if installed via pip
    for sub in ["cublas", "cudnn", "cuda_nvrtc"]:
        p = os.path.join(sys.prefix, "Lib", "site-packages", "nvidia", sub, "bin")
        if os.path.exists(p):
            try:
                os.add_dll_directory(p)
                os.environ["PATH"] = p + os.pathsep + os.environ.get("PATH", "")
            except Exception:
                pass

    has_cuda = False
    try:
        import ctypes
        ctypes.cdll.LoadLibrary("cublas64_12.dll")
        has_cuda = True
    except Exception:
        has_cuda = False

    # Try GPU first if CUDA DLLs exist, otherwise fast CPU int8 with 8 threads
    model = None
    if has_cuda:
        try:
            model = WhisperModel(
                "base",
                device="cuda",
                compute_type="float16",
                download_root=models_dir
            )
            print(json.dumps({"info": "Whisper running on NVIDIA GPU (CUDA float16)"}), file=sys.stderr)
        except Exception as e:
            print(json.dumps({"info": f"CUDA initialization failed ({e}), using CPU"}), file=sys.stderr)
            model = None

    if model is None:
        model = WhisperModel(
            "base",
            device="cpu",
            compute_type="int8",
            cpu_threads=8,
            num_workers=2,
            download_root=models_dir
        )
    
    try:
        segments, info = model.transcribe(
            video_path,
            beam_size=1,
            best_of=1,
            condition_on_previous_text=False,
            word_timestamps=True,
            vad_filter=True,
            vad_parameters=dict(min_silence_duration_ms=500)
        )
    except Exception as e:
        print(json.dumps({"info": f"Transcription error ({e}), retrying on CPU..."}), file=sys.stderr)
        model = WhisperModel(
            "base",
            device="cpu",
            compute_type="int8",
            cpu_threads=8,
            download_root=models_dir
        )
        segments, info = model.transcribe(
            video_path,
            beam_size=1,
            best_of=1,
            condition_on_previous_text=False,
            word_timestamps=True,
            vad_filter=True,
            vad_parameters=dict(min_silence_duration_ms=500)
        )
    
    result = {
        "language": info.language,
        "duration": info.duration,
        "segments": []
    }
    
    for segment in segments:
        seg_data = {
            "start": segment.start,
            "end": segment.end,
            "text": segment.text.strip(),
            "words": []
        }
        if segment.words:
            for word in segment.words:
                seg_data["words"].append({
                    "start": word.start,
                    "end": word.end,
                    "text": word.word.strip(),
                    "confidence": word.probability
                })
        result["segments"].append(seg_data)
        
    print(json.dumps(result, ensure_ascii=False))

if __name__ == "__main__":
    main()