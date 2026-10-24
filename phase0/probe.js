// Phase 0 probe, injected into Vesktop's Discord page over CDP. Logs facts only and
// changes nothing Discord sends: the Go Live connection's SDP, what Chromium offers
// for encoded transforms, and the sender's outbound stats. Read back with
// JSON.stringify(window.__p0).
(() => {
  if (window.__p0) return "already injected";
  const log = (window.__p0 = []);
  const push = (k, v) => log.push({ t: Date.now(), k, v });
  const proto = RTCRtpSender.prototype;
  push("features", {
    RTCRtpScriptTransform: typeof RTCRtpScriptTransform,
    senderCreateEncodedStreams: typeof proto.createEncodedStreams,
    senderGenerateKeyFrame: typeof proto.generateKeyFrame,
    senderTransform: "transform" in proto,
    videoSendCodecs: RTCRtpSender.getCapabilities("video")?.codecs.map(c => `${c.mimeType} ${c.sdpFmtpLine ?? ""}`),
  });
  try {
    const src = `postMessage({ transformer: typeof RTCRtpScriptTransformer,
      generateKeyFrame: typeof RTCRtpScriptTransformer !== "undefined" && "generateKeyFrame" in RTCRtpScriptTransformer.prototype,
      sendKeyFrameRequest: typeof RTCRtpScriptTransformer !== "undefined" && "sendKeyFrameRequest" in RTCRtpScriptTransformer.prototype })`;
    const w = new Worker(URL.createObjectURL(new Blob([src], { type: "text/javascript" })));
    w.onmessage = e => push("worker", e.data);
    w.onerror = e => push("worker-error", String(e.message));
  } catch (e) { push("worker-error", String(e)); }

  const Orig = window.RTCPeerConnection;
  const pcs = (window.__p0pcs = []);
  window.RTCPeerConnection = class extends Orig {
    constructor(...args) {
      super(...args);
      const id = pcs.push(this);
      push("pc", { id, config: JSON.stringify(args[0]) });
      for (const m of ["setLocalDescription", "setRemoteDescription"]) {
        const o = this[m].bind(this);
        this[m] = async (d, ...r) => {
          const res = await o(d, ...r);
          const desc = m === "setLocalDescription" ? this.localDescription : this.remoteDescription;
          push(m, { id, type: desc?.type, sdp: desc?.sdp });
          return res;
        };
      }
      const at = this.addTransceiver.bind(this);
      this.addTransceiver = (t, init) => {
        push("addTransceiver", { id, kind: typeof t === "string" ? t : t?.kind, init: JSON.stringify(init) });
        return at(t, init);
      };
    }
  };

  const gdm = navigator.mediaDevices.getDisplayMedia.bind(navigator.mediaDevices);
  navigator.mediaDevices.getDisplayMedia = async (...a) => {
    const s = await gdm(...a);
    push("getDisplayMedia", { opts: JSON.stringify(a[0]), video: s.getVideoTracks().map(t => t.getSettings()) });
    return s;
  };

  setInterval(async () => {
    for (const [i, pc] of pcs.entries()) {
      if (pc.connectionState === "closed") continue;
      const r = await pc.getStats().catch(() => null);
      if (!r) continue;
      const all = [...r.values()];
      for (const s of all.filter(s => s.type === "outbound-rtp" && s.kind === "video")) {
        const c = r.get(s.codecId);
        push("stats", {
          id: i + 1, ssrc: s.ssrc, codec: c && `${c.mimeType} ${c.sdpFmtpLine ?? ""}`,
          encoder: s.encoderImplementation, powerEfficient: s.powerEfficientEncoder,
          w: s.frameWidth, h: s.frameHeight, fps: s.framesPerSecond,
          targetBitrate: s.targetBitrate, bytesSent: s.bytesSent, keyFramesEncoded: s.keyFramesEncoded,
          pli: s.pliCount, fir: s.firCount, nack: s.nackCount, limit: s.qualityLimitationReason,
          available: all.find(p => p.type === "candidate-pair" && p.nominated)?.availableOutgoingBitrate,
        });
      }
    }
  }, 2000);
  return "injected";
})();
