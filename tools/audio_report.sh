#!/usr/bin/env bash
# Spectrograms and levels for a folder of clips from the `audio_clips` test:
#   cargo test --release audio_clips -- --ignored
#   tools/audio_report.sh target/audio-clips
# Writes <clip>.png beside each WAV and prints peak / RMS levels.
set -euo pipefail
dir="${1:-target/audio-clips}"
for wav in "$dir"/*.wav; do
  name=$(basename "$wav" .wav)
  sox "$wav" -n remix - spectrogram -x 900 -y 300 -z 90 -t "$name" -o "${wav%.wav}.png"
  stats=$(sox "$wav" -n stats 2>&1 | awk '/Pk lev dB/{pk=$4} /RMS lev dB/{rms=$4} END{printf "peak %6s dB  rms %6s dB", pk, rms}')
  printf "%-48s %s\n" "$name" "$stats"
done
