#!/bin/sh
# Post-processing of a clip recorded by the app (live correction is not available:
# MediaRecorder encodes the camera stream as is).
#
#   tools/postprocess.sh clip.mp4 [line_rate_hz] [out.mp4]
#
# line_rate_hz: sensor row readout rate in rows per second. 0 (default) only flattens the
# whole-frame brightness pumping; a positive value also removes the horizontal banding.
# The corrected luma is merged with the original chroma and audio.
set -eu

[ $# -ge 1 ] || { sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }
in=$1
rate=${2:-0}
out=${3:-${in%.*}-fixed.mp4}
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir "$work/in" "$work/out"

fps=$(ffprobe -v error -select_streams v:0 -of csv=p=0 -show_entries stream=r_frame_rate "$in")

echo "extracting frames..." >&2
ffmpeg -v error -i "$in" -vf format=gray "$work/in/frame_%05d.pgm"

echo "correcting (line rate: $rate Hz)..." >&2
cargo run -q --release --manifest-path "$root/Cargo.toml" -p ns-core --example deflicker \
    "$work/in" "$work/out" "$rate"

echo "re-encoding..." >&2
ffmpeg -v error -y -framerate "$fps" -i "$work/out/frame_%05d.pgm" -i "$in" \
    -filter_complex "[1:v]format=yuv444p,setsar=1,extractplanes=u+v[u][v];[0:v]format=gray,setsar=1[y];[y][u][v]mergeplanes=0x001020:yuv444p,format=yuv420p[o]" \
    -map "[o]" -map '1:a?' -c:v libx264 -crf 18 -c:a copy -shortest "$out"

echo "written: $out" >&2
