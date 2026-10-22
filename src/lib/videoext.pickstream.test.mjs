// Mirror of pickStream() in videoext.ts (plain JS so `node` runs it directly). Keep in sync.
import assert from "node:assert";
// replika pickStream (ESM import .vue project butuh bundler; uji logika langsung)
const pickStream = (list) => {
  const score = (s) => {
    if (!s.url) return 99;
    let n = 0;
    if (s.behaviorHints?.notWebReady) n += 3;
    const f = `${s.title ?? ""} ${s.behaviorHints?.filename ?? ""}`;
    if (/\|\^\||[.\s_-]pad([.\s_-]|$)/i.test(f)) n += 2;
    if ((s.behaviorHints?.videoSize ?? 0) > 8e9 || /2160p|\b4k\b|\b8k\b/i.test(f)) n += 1;
    if (/h[.\s]?265|hevc|x265/i.test(f)) n += 1;
    if (!/h[.\s]?264|avc|x264/i.test(f)) n += 1;
    return n;
  };
  const best = [...list].sort((a, b) => score(a) - score(b))[0];
  return best?.url ? best : undefined;
};
// 1: padded h265 vs clean h264 → clean menang walau notWebReady beda urutan
const s = pickStream([
  { name: "PenguPlay", url: "http://a", behaviorHints: { filename: "Obsession.2026.1080p.mp4|^|1080pH.265MP4~1.4Mbps1080ph.265mp4~1.4mbps.pad-MovieBox" } },
  { name: "clean264", url: "http://b", behaviorHints: { filename: "Movie.2026.1080p.WEB.h264.mp4" } },
]);
assert.equal(s.name, "clean264");
// 2: hanya padded → tetap dikembalikan (upaya terakhir)
const s2 = pickStream([{ name: "pad", url: "http://a", behaviorHints: { filename: "x.1080p.mp4.pad-MovieBox" } }]);
assert.equal(s2.name, "pad");
// 3: torrent tanpa url dilewati kalau ada url
const s3 = pickStream([{ infoHash: "abc" }, { name: "url", url: "http://c", behaviorHints: { filename: "x.hevc.mkv" } }]);
assert.equal(s3.name, "url");
// 4: semua tanpa url → undefined
assert.equal(pickStream([{ infoHash: "abc" }]), undefined);
// 5: kosong
assert.equal(pickStream([]), undefined);
// 6: h264 notWebReady vs hevc clean → hevc clean menang (bisa decode, libav ada)
const s6 = pickStream([
  { name: "h264-nwr", url: "http://a", behaviorHints: { notWebReady: true, filename: "x.h264.mp4" } },
  { name: "hevc-clean", url: "http://b", behaviorHints: { filename: "x.h265.mkv" } },
]);
assert.equal(s6.name, "hevc-clean");
// 7: 4K besar tanpa token codec kalah dari 1080p h264 (bug sebelumnya)
const s7 = pickStream([
  { name: "4K-DV", url: "http://a", behaviorHints: { filename: "Obsession.2026.2160p.iT.WEB.DL.UNRATED.DV.HDR10.MULTiBen.The.Men.mp4" } },
  { name: "1080p-h264", url: "http://b", behaviorHints: { filename: "Obsession.2026.WEBDL.1080p.AAC.H264.mp4" } },
]);
assert.equal(s7.name, "1080p-h264");
console.log("semua 7 kasus lulus");
