// Loopback test in the page: a MediaStreamTrackGenerator feeds an H.264 sender with
// encoded streams; logs each encoded frame's metadata against the VideoFrame timestamp.
(async () => {
  const W = 1280, H = 720;
  const gen = new MediaStreamTrackGenerator({ kind: "video" });
  gen.contentHint = "detail";
  const writer = gen.writable.getWriter();
  const cfg = { encodedInsertableStreams: true };
  const a = new RTCPeerConnection(cfg), b = new RTCPeerConnection(cfg);
  a.onicecandidate = e => e.candidate && b.addIceCandidate(e.candidate);
  b.onicecandidate = e => e.candidate && a.addIceCandidate(e.candidate);
  const tr = a.addTransceiver(gen, { direction: "sendonly" });
  const h264 = RTCRtpSender.getCapabilities("video").codecs.filter(c => /42e01f/.test(c.sdpFmtpLine ?? "") && /packetization-mode=1/.test(c.sdpFmtpLine));
  tr.setCodecPreferences(h264);
  const { readable, writable } = tr.sender.createEncodedStreams();
  const log = []; const writes = [];
  readable.pipeThrough(new TransformStream({ transform(f, c) {
    const m = f.getMetadata();
    log.push({ at: performance.now(), type: f.type, ts: f.timestamp, size: f.data.byteLength, meta: Object.fromEntries(Object.entries(m).filter(([k]) => k !== "dependencies" && k !== "decodeTargetIndications")) });
    c.enqueue(f);
  } })).pipeTo(writable);
  b.ontrack = e => { const r = e.receiver.createEncodedStreams(); r.readable.pipeTo(r.writable); };
  await a.setLocalDescription(); await b.setRemoteDescription(a.localDescription);
  await b.setLocalDescription(); await a.setRemoteDescription(b.localDescription);
  const buf = new Uint8Array(W * H * 3 / 2); buf.fill(16, 0, W * H); buf.fill(128, W * H);
  const sent = [];
  for (let i = 0; i < 90; i++) {
    const ts = 5_000_000 + i * 33_333 + (i === 40 ? 0 : 0);
    if (i === 50 || i === 51) { await new Promise(r => setTimeout(r, 33)); continue; } // gap: two frames never written
    writes.push(performance.now());
    await writer.write(new VideoFrame(buf, { format: "I420", codedWidth: W, codedHeight: H, timestamp: ts }));
    sent.push(ts);
    await new Promise(r => setTimeout(r, 33));
  }
  await new Promise(r => setTimeout(r, 500));
  const s = [...(await a.getStats()).values()].find(s => s.type === "outbound-rtp");
  a.close(); b.close(); gen.stop();
  // Residual of each encoded frame's rtp against the nearest write time, after the first pairing.
  const t0 = writes[0], r0 = log[0].meta.rtpTimestamp;
  const res = log.map(f => { const t = t0 + ((f.meta.rtpTimestamp - r0) >>> 0) / 90; let best = 1e9, k = -1; writes.forEach((w, i) => { if (Math.abs(w - t) < Math.abs(best)) { best = w - t; k = i; } }); return [k, +best.toFixed(2)]; });
  const lat = log.map((f, i) => +(f.at - writes[res[i][0]]).toFixed(1));
  return JSON.stringify({ residual: res, encodeLatency: lat, sent: sent.length, encoded: log.length, first: log.slice(0, 3), around: log.slice(46, 52), stats: { w: s.frameWidth, h: s.frameHeight, enc: s.encoderImplementation, kf: s.keyFramesEncoded, frames: s.framesEncoded } });
})()
