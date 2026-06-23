"""
TTS service using sherpa-onnx VITS Chinese model.
Replaces edge-tts with local, offline, Apache 2.0 licensed TTS.

Usage:
  python tts_service.py --tokens <tokens.txt> --model <model.onnx> --lexicon <lexicon.txt> --dict-dir <dict/>
       --text <text> --output <wav_path> [--speaker <0-4>] [--speed <0.5-2.0>]

Reads text from --text argument or stdin, generates speech audio,
writes WAV file to --output path.
"""

import sys
import os
import argparse
import wave
import numpy as np
import sherpa_onnx

sys.stdout.reconfigure(encoding='utf-8')

# Global TTS engine instance (lazy loaded)
_tts = None
_tts_config = None


def get_tts(model, tokens, lexicon, dict_dir):
    global _tts, _tts_config
    current_config = (model, tokens, lexicon, dict_dir)
    if _tts is None or _tts_config != current_config:
        tts_config = sherpa_onnx.OfflineTtsConfig(
            model=sherpa_onnx.OfflineTtsModelConfig(
                vits=sherpa_onnx.OfflineTtsVitsModelConfig(
                    model=model,
                    tokens=tokens,
                    lexicon=lexicon,
                    dict_dir=dict_dir,
                ),
            ),
        )
        _tts = sherpa_onnx.OfflineTts(tts_config)
        _tts_config = current_config
    return _tts


def main():
    parser = argparse.ArgumentParser(description='TTS with sherpa-onnx VITS')
    parser.add_argument('--tokens', required=True, help='Path to tokens.txt')
    parser.add_argument('--model', required=True, help='Path to model.onnx')
    parser.add_argument('--lexicon', required=True, help='Path to lexicon.txt')
    parser.add_argument('--dict-dir', required=True, help='Path to dict/ directory')
    parser.add_argument('--text', default=None, help='Text to synthesize (or read from stdin)')
    parser.add_argument('--output', required=True, help='Path to output WAV file')
    parser.add_argument('--speaker', type=int, default=0, help='Speaker ID (0-4)')
    parser.add_argument('--speed', type=float, default=1.1, help='Speech speed')
    parser.add_argument('--num-threads', type=int, default=4, help='Number of CPU threads')
    args = parser.parse_args()

    # Get text (UTF-8 encoded from Rust via stdin bytes)
    text = args.text
    if text is None:
        raw = sys.stdin.buffer.read()
        text = raw.decode('utf-8').strip()
    if not text:
        print("ERROR: no text provided", file=sys.stderr)
        sys.exit(1)

    print(f"TTS received text ({len(text)} chars): {text[:80]}", file=sys.stderr)

    # Verify files
    for path_name, path_val in [('model', args.model), ('tokens', args.tokens),
                                 ('lexicon', args.lexicon)]:
        if not os.path.exists(path_val):
            print(f"ERROR: {path_name} file not found: {path_val}", file=sys.stderr)
            sys.exit(1)

    # Get TTS engine
    tts = get_tts(args.model, args.tokens, args.lexicon, args.dict_dir)

    # Generate audio
    audio = tts.generate(text=text, sid=args.speaker, speed=args.speed)

    # Write WAV
    samples = np.array(audio.samples, dtype=np.float32)
    with wave.open(args.output, 'wb') as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(audio.sample_rate)
        w.writeframes((samples * 32767).clip(-32768, 32767).astype(np.int16).tobytes())

    sys.stdout.write(f'OK {len(audio.samples)} samples rate={audio.sample_rate}')


if __name__ == '__main__':
    main()
