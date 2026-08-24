#!/usr/bin/env bash
# Dev-only helper: regenerate deterministic audio fixtures for tests/audio_codecs.rs.
# Requires ffmpeg on PATH. NOT a runtime or test-time dependency of tui-explorer;
# generated binaries are committed. Self-synthesized sine tones: redistribution-safe.
set -euo pipefail

OUT="$(cd "$(dirname "$0")/.." && pwd)/tests/fixtures/audio"
mkdir -p "$OUT"

# Deterministic 440 Hz sine, 1 s, mono where noted, loudness-normalized off (no metadata).
# ≤50 KB each; -b:a / sample rates chosen to stay small while remaining decodable.
common=(-hide_banner -loglevel error -y -f lavfi -i "sine=frequency=440:duration=1:sample_rate=22050" -map_metadata -1 -fflags +bitexact -flags:a +bitexact)
gen() {
  local name="$1"; shift
  ffmpeg "${common[@]}" "$@" "$OUT/$name"
  echo "wrote $name ($(wc -c <"$OUT/$name") bytes)"
}

gen wav_pcm.wav      -c:a pcm_s16le
gen tone.flac        -c:a flac -sample_fmt s16
gen tone.mp3         -c:a libmp3lame -b:a 96k -write_xing 0 -id3v2_version 0
gen tone.ogg         -c:a libvorbis -qscale:a 2
gen tone_aac.m4a     -c:a aac -b:a 64k -movflags +faststart
gen tone_alac.m4a    -c:a alac
gen tone.opus        -c:a libopus -b:a 48k -application voip
gen tone.wma         -c:a wmav2 -b:a 64k
gen tone.aiff        -c:a pcm_s16be
gen tone_sowt.aiff   -c:a pcm_s16le   # 'sowt' little-endian in AIFF container

# Truncated/corrupt variants for bounded-error tests (mid-frame cut at ~40%).
for f in wav_pcm.wav tone.flac tone.mp3 tone.ogg tone_aac.m4a tone_alac.m4a; do
  size=$(wc -c <"$OUT/$f")
  cut=$((size * 2 / 5))
  head -c "$cut" "$OUT/$f" >"$OUT/${f%.*}_trunc.${f##*.}"
done

echo "all fixtures in $OUT:"
ls -l "$OUT"
