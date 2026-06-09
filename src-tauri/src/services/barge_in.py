"""
Barge-in monitor — listens for user speech during TTS playback.
Lightweight VAD-only script: detects speech energy, does NOT do full ASR.
Exits with "SPEECH" if user is talking, or "SILENCE" on timeout.

Usage:
  python barge_in.py [--threshold 0.005] [--confirm-frames 3] [--timeout 10]
"""

import sys
import argparse
import numpy as np
import sounddevice as sd

sys.stdout.reconfigure(encoding='utf-8')


def find_best_input_device():
    """Find available input device preferring DirectSound/WASAPI over MME.

    Returns (device_id, sample_rate). DirectSound/WASAPI backends typically
    don't support 16kHz, so we use the device's native rate.
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


def main():
    parser = argparse.ArgumentParser(description='Barge-in speech detector')
    parser.add_argument('--threshold', type=float, default=0.005,
                        help='Volume threshold for speech detection')
    parser.add_argument('--confirm-frames', type=int, default=3,
                        help='Consecutive speech frames to confirm (avoids false triggers)')
    parser.add_argument('--timeout', type=float, default=10.0,
                        help='Max monitoring duration in seconds')
    args = parser.parse_args()

    sr = 16000
    best_device, device_sr = find_best_input_device()
    chunk_samples = int(device_sr * 0.03)  # 30ms frames for fast response

    stream = sd.InputStream(samplerate=device_sr, channels=2, dtype='float32', device=best_device)
    stream.start()

    speech_count = 0
    max_chunks = int(args.timeout * 1000 / 30)  # 30ms per chunk

    for _ in range(max_chunks):
        chunk_2ch, _ = stream.read(chunk_samples)
        # Intel mic array: take ch0
        chunk = chunk_2ch[:, 0]
        peak = float(np.max(np.abs(chunk)))

        if peak > args.threshold:
            speech_count += 1
        else:
            speech_count = 0

        if speech_count >= args.confirm_frames:
            stream.stop()
            stream.close()
            sys.stdout.write("SPEECH")
            return

    stream.stop()
    stream.close()
    sys.stdout.write("SILENCE")


if __name__ == '__main__':
    main()
