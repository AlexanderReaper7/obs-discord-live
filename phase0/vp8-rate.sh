#!/usr/bin/env bash
# vp8-rate.sh NAME, in a directory holding src.y4m (or SRC=file), [extra ffmpeg output options...]: encodes src.y4m with the plugin's libvpx
# options at 800 kbps CBR and prints kbps per second of video.
set -eu
name=$1; shift
ffmpeg -hide_banner -loglevel error -y -i ${SRC:-src.y4m} -c:v libvpx -deadline realtime -cpu-used 8 \
  -lag-in-frames 0 -max-intra-rate 900 -g 240 -b:v 800k -minrate 800k -maxrate 800k -bufsize 400k \
  "$@" $name.ivf
ffprobe -v error -select_streams v -show_entries packet=size -of csv=p=0 $name.ivf |
  awk -v n=$name '{s+=$1; i++; if (i%30==0) {printf "%d ", s*8/1000; s=0}} END {print " <- " n}'
