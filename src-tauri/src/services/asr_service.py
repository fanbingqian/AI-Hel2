"""
ASR service using sherpa-onnx zipformer2 CTC (online/streaming).
Records from microphone → streams audio chunks to online recognizer → returns text.

Usage:
  One-shot:  python asr_service.py --tokens <tokens.txt> --model <model.onnx> [--stop-file <path>]
  Daemon:    python asr_service.py --tokens <tokens.txt> --model <model.onnx> --daemon

Daemon mode loads the model once then loops reading RECORD commands from stdin,
returning RESULT lines on stdout. This eliminates ~1.7s of model-loading overhead
on every request.

Auto-stops on silence detection or stop file creation.
"""

import sys
import os
import argparse
import numpy as np
import sounddevice as sd
import sherpa_onnx
from scipy import signal as scipy_signal

sys.stdout.reconfigure(encoding='utf-8')


def find_best_input_device():
    """Find available input device preferring DirectSound/WASAPI over MME.

    Intel Smart Sound mic arrays get severely attenuated (30-35x) through the
    MME backend in PortAudio. DirectSound and WASAPI give proper levels.
    """
    devices = sd.query_devices()
    hostapis = {i: sd.query_hostapis(i) for i in range(len(sd.query_hostapis()))}

    default_in = sd.default.device[0]
    if default_in is not None and default_in < len(devices):
        default_name = devices[default_in]['name']
    else:
        default_name = None

    for pref in ['Windows WASAPI', 'Windows DirectSound']:
        for i, d in enumerate(devices):
            api = hostapis.get(d['hostapi'])
            if d['max_input_channels'] > 0 and api and pref in api['name']:
                if default_name and d['name'] == default_name:
                    return i, d['default_samplerate']
        for i, d in enumerate(devices):
            api = hostapis.get(d['hostapi'])
            if d['max_input_channels'] > 0 and api and pref in api['name']:
                return i, d['default_samplerate']

    return default_in, 44100


def record_audio(best_device, device_sr, gain_linear, max_seconds, stop_file):
    """Record audio from mic, return (audio_array, has_speech).

    If stop_file is provided, only that file triggers stop (no auto-silence).
    Without stop_file, auto-stops on 1.5s of silence.
    """
    buf = []
    silence_count = 0
    silence_frames = 5  # 1.5s of silence triggers stop
    threshold = 0.005
    has_speech = False

    chunk_samples_device = int(device_sr * 0.3)
    max_samples = int(device_sr * max_seconds)

    stream = sd.InputStream(samplerate=device_sr, channels=2, dtype='float32', device=best_device)
    stream.start()

    total = 0
    while total < max_samples:
        chunk_2ch, _ = stream.read(chunk_samples_device)
        chunk = chunk_2ch[:, 0]

        peak = float(np.max(np.abs(chunk)))
        if peak > threshold:
            has_speech = True
            silence_count = 0
        elif has_speech:
            silence_count += 1

        boosted = np.clip(chunk * gain_linear, -1.0, 1.0)
        buf.append(boosted)
        total += len(boosted)

        if stop_file:
            # Push-and-hold mode: only stop file triggers stop
            if os.path.exists(stop_file):
                break
        else:
            # Listen-once mode: auto-stop on silence
            if silence_count >= silence_frames:
                break

    stream.stop()
    stream.close()

    if stop_file:
        try:
            os.remove(stop_file)
        except OSError:
            pass

    if not buf or not has_speech:
        return None, False

    audio = np.concatenate(buf).astype(np.float32).flatten()
    return audio, True


def recognize_audio(recognizer, audio, device_sr, sr=16000):
    """Run recognition on audio array. Returns transcribed text."""
    if device_sr != sr:
        target_len = int(len(audio) * sr / device_sr)
        audio = scipy_signal.resample(audio, target_len).astype(np.float32)

    s = recognizer.create_stream()
    s.accept_waveform(sr, audio)
    s.input_finished()

    while recognizer.is_ready(s):
        recognizer.decode_stream(s)

    return recognizer.get_result(s).strip()


def run_daemon(args):
    """Daemon mode: load model once, process multiple RECORD commands.

    Protocol (stdin → stdout):
      READY                  ← daemon signals it's ready
      RECORD <secs> <stop>   → start recording (stop file optional, use - for none)
      RESULT <text>          ← transcribed text (empty if no speech)
      ERROR <message>        ← on failure
    """
    sr = 16000

    # Load model once
    recognizer = sherpa_onnx.OnlineRecognizer.from_zipformer2_ctc(
        model=args.model,
        tokens=args.tokens,
        num_threads=args.num_threads,
        sample_rate=sr,
        provider='cpu',
    )

    # Detect device once
    best_device, device_sr = find_best_input_device()
    gain_linear = 10 ** (6 / 20)

    sys.stdout.write("READY\n")
    sys.stdout.flush()

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        if line == "STOP":
            break

        parts = line.split(maxsplit=2)
        cmd = parts[0]

        if cmd == "RECORD":
            max_seconds = float(parts[1]) if len(parts) > 1 else 30.0
            stop_file = parts[2] if len(parts) > 2 else None
            if stop_file == "-":
                stop_file = None

            try:
                audio, has_speech = record_audio(
                    best_device, device_sr, gain_linear,
                    max_seconds, stop_file,
                )
                if not has_speech:
                    sys.stdout.write("RESULT \n")
                else:
                    text = recognize_audio(recognizer, audio, device_sr, sr)
                    sys.stdout.write(f"RESULT {text}\n")
            except Exception as e:
                sys.stdout.write(f"ERROR {e}\n")
            sys.stdout.flush()


def main():
    parser = argparse.ArgumentParser(description='ASR with sherpa-onnx zipformer2 CTC')
    parser.add_argument('--tokens', required=True, help='Path to tokens.txt')
    parser.add_argument('--model', required=True, help='Path to model.onnx')
    parser.add_argument('--stop-file', default=None, help='Path to stop signal file')
    parser.add_argument('--num-threads', type=int, default=4, help='Number of CPU threads')
    parser.add_argument('--max-duration', type=float, default=30.0,
                        help='Maximum recording duration in seconds')
    parser.add_argument('--daemon', action='store_true',
                        help='Run in daemon mode (persistent process, stdin/stdout protocol)')
    args = parser.parse_args()

    if not os.path.exists(args.tokens):
        print(f"ERROR: tokens file not found: {args.tokens}", file=sys.stderr)
        sys.exit(1)
    if not os.path.exists(args.model):
        print(f"ERROR: model file not found: {args.model}", file=sys.stderr)
        sys.exit(1)

    if args.daemon:
        run_daemon(args)
        return

    # One-shot mode (backward compatible)
    sr = 16000
    best_device, device_sr = find_best_input_device()
    gain_linear = 10 ** (6 / 20)

    audio, has_speech = record_audio(
        best_device, device_sr, gain_linear,
        args.max_duration, args.stop_file,
    )
    if not has_speech:
        sys.stdout.write("")
        return

    recognizer = sherpa_onnx.OnlineRecognizer.from_zipformer2_ctc(
        model=args.model,
        tokens=args.tokens,
        num_threads=args.num_threads,
        sample_rate=sr,
        provider='cpu',
    )
    text = recognize_audio(recognizer, audio, device_sr, sr)
    sys.stdout.write(text)


if __name__ == '__main__':
    main()
