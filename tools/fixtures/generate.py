#!/usr/bin/env python3
"""Generate deterministic synthetic fixtures into fixtures/synth (DESIGN.md 7.2).

Usage: python3 tools/fixtures/generate.py [--quick] [--out DIR] [--only images|video|audio|pdf|office|archives|text]
Video needs ffmpeg on PATH; everything else needs only Pillow.
"""
import argparse, io, json, os, random, shutil, struct, subprocess, sys, zipfile, zlib
from pathlib import Path

try:
    from PIL import Image, ImageDraw
except ImportError:
    sys.exit("Pillow is required: pip install pillow")

ROOT = Path(__file__).resolve().parents[2]
ap = argparse.ArgumentParser()
ap.add_argument("--out", default=str(ROOT / "fixtures" / "synth"))
ap.add_argument("--quick", action="store_true", help="skip the biggest fixtures (15 min video, 70 min audio, 48 MP)")
ap.add_argument("--only", choices=["images", "video", "audio", "pdf", "office", "archives", "text"], action="append")
args = ap.parse_args()
OUT = Path(args.out)
OUT.mkdir(parents=True, exist_ok=True)
want = lambda k: not args.only or k in args.only
FFMPEG = shutil.which("ffmpeg")
manifest = {}


def note(name, **meta):
    p = OUT / name
    manifest[name] = {"bytes": p.stat().st_size if p.exists() else None, **meta}
    print(f"  {name:45s} {manifest[name]['bytes']}")


def photo_like(w, h, seed):
    """Gradient plus noise plus a few shapes: compresses like a photo, not like testsrc."""
    rnd = random.Random(seed)
    img = Image.effect_noise((w, h), 48).convert("RGB")
    grad = Image.linear_gradient("L").resize((w, h))
    img = Image.blend(img, Image.merge("RGB", (grad, grad.rotate(90), grad.transpose(Image.FLIP_LEFT_RIGHT))), 0.55)
    d = ImageDraw.Draw(img)
    for _ in range(60):
        x, y = rnd.randrange(w), rnd.randrange(h)
        r = rnd.randrange(w // 40, w // 6)
        d.ellipse([x - r, y - r, x + r, y + r], fill=tuple(rnd.randrange(256) for _ in range(3)))
    return img


def screenshot_like(w, h):
    img = Image.new("RGB", (w, h), (246, 244, 240))
    d = ImageDraw.Draw(img)
    d.rectangle([0, 0, w, h // 12], fill=(109, 74, 255))
    for i in range(14):
        y = h // 10 + i * (h // 18)
        d.rectangle([w // 20, y, w - w // 20, y + h // 40], fill=(255, 255, 255), outline=(214, 208, 200))
        d.text((w // 18, y + 4), "The quick brown fox jumps over the lazy dog 0123456789", fill=(28, 25, 36))
    for i in range(8):
        d.rectangle([w // 20 + i * (w // 9), h - h // 6, w // 20 + i * (w // 9) + w // 12, h - h // 20], fill=(47, 191, 113) if i % 2 else (240, 86, 106))
    return img


if want("images"):
    print("images")
    sizes = [("photo_1mp.jpg", 1280, 800), ("photo_12mp.jpg", 4000, 3000)] + ([] if args.quick else [("photo_48mp.jpg", 8000, 6000)])
    for name, w, h in sizes:
        photo_like(w, h, 1).save(OUT / name, quality=95, subsampling=0)
        note(name, kind="image", klass="photo", w=w, h=h)
    screenshot_like(3840, 2160).save(OUT / "screenshot_4k.png", optimize=False)
    note("screenshot_4k.png", kind="image", klass="graphic")
    # transparent logo (graphic + alpha)
    logo = Image.new("RGBA", (1200, 800), (0, 0, 0, 0))
    d = ImageDraw.Draw(logo)
    d.rounded_rectangle([100, 100, 1100, 700], radius=120, fill=(109, 74, 255, 255))
    d.rounded_rectangle([400, 300, 800, 500], radius=40, fill=(255, 255, 255, 255))
    logo.save(OUT / "logo_transparent.png")
    note("logo_transparent.png", kind="image", klass="graphic", alpha=True)
    # transparent photo cut-out (photo + alpha)
    cut = photo_like(2000, 1500, 2).convert("RGBA")
    mask = Image.new("L", (2000, 1500), 0)
    ImageDraw.Draw(mask).ellipse([200, 100, 1800, 1400], fill=255)
    cut.putalpha(mask)
    cut.save(OUT / "cutout_transparent.png")
    note("cutout_transparent.png", kind="image", klass="photo", alpha=True)
    # 16-bit PNG
    arr = Image.effect_noise((1600, 1200), 60).convert("I")
    arr = arr.point(lambda v: v * 256)
    arr.save(OUT / "sixteen_bit.png")
    note("sixteen_bit.png", kind="image", bits=16)
    # EXIF orientation 1..8 on a non-square photo with a marker in one corner
    base = photo_like(800, 600, 3)
    ImageDraw.Draw(base).rectangle([0, 0, 120, 120], fill=(255, 0, 0))
    for o in range(1, 9):
        ex = Image.Exif()
        ex[0x0112] = o
        base.save(OUT / f"orientation_{o}.jpg", quality=90, exif=ex.tobytes())
        note(f"orientation_{o}.jpg", kind="image", orientation=o)
    # animated GIF, about 15 MB when not quick
    frames = []
    n = 40 if args.quick else 240
    for i in range(n):
        f = photo_like(640, 360, 100 + i)
        frames.append(f.convert("P", palette=Image.ADAPTIVE, colors=256))
    frames[0].save(OUT / "anim_big.gif", save_all=True, append_images=frames[1:], duration=40, loop=0, optimize=False)
    note("anim_big.gif", kind="animated", frames=n)
    photo_like(1600, 1200, 4).save(OUT / "photo.webp", quality=90)
    note("photo.webp", kind="image")
    photo_like(1600, 1200, 5).save(OUT / "photo.bmp")
    note("photo.bmp", kind="image")
    photo_like(1600, 1200, 6).save(OUT / "photo.tiff")
    note("photo.tiff", kind="image")
    small = photo_like(320, 240, 7)
    small.save(OUT / "tiny_already_fits.jpg", quality=80)
    note("tiny_already_fits.jpg", kind="image")
    with open(OUT / "corrupt.jpg", "wb") as f:
        f.write(b"\xff\xd8\xff\xe0" + os.urandom(20_000))
    note("corrupt.jpg", kind="image", corrupt=True)

if want("video"):
    print("video")
    if not FFMPEG:
        print("  ffmpeg not on PATH: skipping video fixtures")
    else:
        def ff(name, *a, **meta):
            cmd = [FFMPEG, "-v", "error", "-y", *a, str(OUT / name)]
            subprocess.run(cmd, check=True)
            note(name, kind="video", **meta)

        SINE = ["-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000"]
        X264 = ["-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p"]
        AAC = ["-c:a", "aac", "-b:a", "128k"]
        for (res, w, h) in [("480p", 854, 480), ("720p", 1280, 720), ("1080p", 1920, 1080), ("1440p", 2560, 1440), ("2160p", 3840, 2160)]:
            for fps in (30, 60):
                if fps == 60 and res in ("480p",):
                    continue
                ff(f"v_{res}_{fps}fps_10s.mp4", "-f", "lavfi", "-i", f"testsrc2=size={w}x{h}:rate={fps}", *SINE, "-t", "10", *X264, *AAC, "-shortest", res=res, fps=fps, dur=10)
        ff("v_720p_30fps_2min.mp4", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30", *SINE, "-t", "120", *X264, *AAC, "-shortest", dur=120)
        ff("v_1080p_60fps_2min.mp4", "-f", "lavfi", "-i", "testsrc2=size=1920x1080:rate=60", *SINE, "-t", "120", *X264, *AAC, "-shortest", dur=120)
        if not args.quick:
            ff("v_1080p_30fps_15min.mp4", "-f", "lavfi", "-i", "testsrc2=size=1920x1080:rate=30", *SINE, "-t", "900", *X264, "-crf", "23", *AAC, "-shortest", dur=900)
        ff("v_portrait_1080x1920_10s.mp4", "-f", "lavfi", "-i", "testsrc2=size=1080x1920:rate=30", *SINE, "-t", "10", *X264, *AAC, "-shortest", portrait=True)
        ff("v_noaudio_10s.mp4", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30", "-t", "10", *X264, "-an", audio=False)
        ff("v_two_audio_tracks.mkv", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30", *SINE, "-f", "lavfi", "-i", "sine=frequency=880:sample_rate=48000", "-t", "10",
           "-map", "0", "-map", "1", "-map", "2", "-metadata:s:a:0", "title=Game", "-metadata:s:a:1", "title=Mic", *X264, "-c:a", "libvorbis", "-shortest", audio_tracks=2)
        # rotation tag: encoded landscape, display matrix says rotate 90 -> portrait display
        ff("v_rotated_90.mp4", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30", *SINE, "-t", "10", *X264, *AAC, "-shortest", "-metadata:s:v:0", "rotate=90", rotation=90)
        # HDR: HLG and PQ tagged clips (zscale to bt2020)
        for trc, name in (("arib-std-b67", "v_hdr_hlg.mp4"), ("smpte2084", "v_hdr_pq.mp4")):
            try:
                ff(name, "-f", "lavfi", "-i", "testsrc2=size=1920x1080:rate=30", *SINE, "-t", "10",
                   "-vf", f"zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt2020,zscale=t={trc}:m=2020_ncl:r=tv,format=yuv420p10le",
                   "-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p10le",
                   "-color_primaries", "bt2020", "-color_trc", trc, "-colorspace", "bt2020nc", *AAC, "-shortest", hdr=trc)
            except subprocess.CalledProcessError:
                print(f"  {name}: this ffmpeg lacks zscale or 10-bit x264; skipped")
        # VFR: variable frame timing from setpts
        ff("v_vfr.mp4", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=60", *SINE, "-t", "10", "-vf", "setpts='PTS+if(mod(N,7),0,0.5/TB)'", "-fps_mode", "vfr", *X264, *AAC, "-shortest", vfr=True)
        ff("v_vp9.webm", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30", *SINE, "-t", "10", "-c:v", "libvpx-vp9", "-b:v", "2M", "-c:a", "libopus", "-shortest", codec="vp9")
        with open(OUT / "v_truncated.mp4", "wb") as f:
            f.write((OUT / "v_720p_30fps_10s.mp4").read_bytes()[:200_000])
        note("v_truncated.mp4", kind="video", corrupt=True)
        shutil.copy(OUT / "v_portrait_1080x1920_10s.mp4", OUT / "Grandkids 🎂 誕生日 (final).mp4")
        note("Grandkids 🎂 誕生日 (final).mp4", kind="video", unicode_name=True)

if want("audio"):
    print("audio")
    if not FFMPEG:
        print("  ffmpeg not on PATH: skipping audio fixtures")
    else:
        def fa(name, secs, *a, **meta):
            subprocess.run([FFMPEG, "-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=44100", "-f", "lavfi", "-i", "anoisesrc=color=pink:sample_rate=44100:amplitude=0.1",
                            "-filter_complex", "[0][1]amix=inputs=2", "-t", str(secs), *a, str(OUT / name)], check=True)
            note(name, kind="audio", dur=secs, **meta)
        for secs, tag in [(10, "10s"), (300, "5min")] + ([] if args.quick else [(4200, "70min")]):
            fa(f"a_{tag}.wav", secs, "-c:a", "pcm_s16le")
            fa(f"a_{tag}.flac", secs, "-c:a", "flac")
            fa(f"a_{tag}.mp3", secs, "-c:a", "libmp3lame", "-b:a", "320k")
            fa(f"a_{tag}.ogg", secs, "-c:a", "libvorbis", "-q:a", "6")
        fa("a_stereo_10s.m4a", 10, "-c:a", "aac", "-b:a", "256k")
        fa("a_mono_10s.wav", 10, "-ac", "1", "-c:a", "pcm_s16le", mono=True)

if want("pdf"):
    print("pdf")
    pages = [photo_like(2000, 1500, 10 + i).convert("RGB") for i in range(5 if args.quick else 12)]
    pages[0].save(OUT / "scan_photos.pdf", save_all=True, append_images=pages[1:], resolution=200.0, quality=95)
    note("scan_photos.pdf", kind="pdf", pages=len(pages))
    # vector-only: text drawn as PDF content streams written by hand
    def vector_pdf(path, n_pages=3):
        objs = []
        def add(s):
            objs.append(s); return len(objs)
        font = add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
        page_ids = []
        content_ids = []
        for p in range(n_pages):
            lines = [f"BT /F1 14 Tf 50 {780 - 18 * i} Td (Page {p + 1} line {i}: the quick brown fox jumps over the lazy dog) Tj ET".encode() for i in range(40)]
            body = b"\n".join(lines)
            content_ids.append(add(b"<< /Length %d >>\nstream\n" % len(body) + body + b"\nendstream"))
        pages_id = len(objs) + n_pages + 1
        for p in range(n_pages):
            page_ids.append(add(f"<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 612 792] /Contents {content_ids[p]} 0 R /Resources << /Font << /F1 {font} 0 R >> >> >>".encode()))
        kids = " ".join(f"{i} 0 R" for i in page_ids)
        assert add(f"<< /Type /Pages /Kids [{kids}] /Count {n_pages} >>".encode()) == pages_id
        catalog = add(f"<< /Type /Catalog /Pages {pages_id} 0 R >>".encode())
        out = io.BytesIO(); out.write(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
        offsets = []
        for i, o in enumerate(objs, 1):
            offsets.append(out.tell()); out.write(f"{i} 0 obj\n".encode() + o + b"\nendobj\n")
        xref = out.tell()
        out.write(f"xref\n0 {len(objs) + 1}\n0000000000 65535 f \n".encode())
        for off in offsets:
            out.write(f"{off:010d} 00000 n \n".encode())
        out.write(f"trailer\n<< /Size {len(objs) + 1} /Root {catalog} 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode())
        path.write_bytes(out.getvalue())
    vector_pdf(OUT / "vector_text.pdf")
    note("vector_text.pdf", kind="pdf", pages=3)
    # "encrypted": a PDF whose trailer has /Encrypt (the planner must refuse by detecting the key, not by decrypting)
    enc = (OUT / "vector_text.pdf").read_bytes().replace(b"/Root", b"/Encrypt << /Filter /Standard /V 1 /R 2 /O <00> /U <00> /P -1 >> /Root")
    (OUT / "encrypted_marker.pdf").write_bytes(enc)
    note("encrypted_marker.pdf", kind="pdf", encrypted=True)

if want("office"):
    print("office")
    def ooxml(path, kind, media):
        ct = {"docx": "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
              "pptx": "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"}[kind]
        main = {"docx": "word/document.xml", "pptx": "ppt/presentation.xml"}[kind]
        media_dir = {"docx": "word/media", "pptx": "ppt/media"}[kind]
        with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
            defaults = '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="jpeg" ContentType="image/jpeg"/><Default Extension="png" ContentType="image/png"/><Default Extension="mp4" ContentType="video/mp4"/>'
            z.writestr("[Content_Types].xml", f'<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">{defaults}<Override PartName="/{main}" ContentType="{ct}"/></Types>')
            z.writestr("_rels/.rels", f'<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="{main}"/></Relationships>')
            rels = "".join(f'<Relationship Id="rId{i + 10}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/{n}"/>' for i, (n, _) in enumerate(media))
            z.writestr(main.rsplit("/", 1)[0] + "/_rels/" + main.rsplit("/", 1)[1] + ".rels", f'<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>')
            body = {"docx": '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Synthetic fixture</w:t></w:r></w:p></w:body></w:document>',
                    "pptx": '<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"/>'}[kind]
            z.writestr(main, '<?xml version="1.0" encoding="UTF-8"?>' + body)
            for n, data in media:
                z.writestr(f"{media_dir}/{n}", data)
    def jpeg_bytes(seed, w=3000, h=2000):
        b = io.BytesIO(); photo_like(w, h, seed).save(b, "JPEG", quality=95); return b.getvalue()
    def png_bytes():
        b = io.BytesIO(); screenshot_like(1920, 1080).save(b, "PNG"); return b.getvalue()
    ooxml(OUT / "report.docx", "docx", [(f"image{i}.jpeg", jpeg_bytes(20 + i)) for i in range(6)] + [("image9.png", png_bytes())])
    note("report.docx", kind="office")
    media = [(f"image{i}.jpeg", jpeg_bytes(40 + i)) for i in range(10 if args.quick else 25)]
    if (OUT / "v_720p_30fps_10s.mp4").exists():
        media.append(("media1.mp4", (OUT / "v_720p_30fps_10s.mp4").read_bytes()))
    ooxml(OUT / "deck.pptx", "pptx", media)
    note("deck.pptx", kind="office")

if want("archives"):
    print("archives")
    srcs = sorted(p for p in OUT.glob("photo_*.jpg"))
    if srcs:
        with zipfile.ZipFile(OUT / "photos.zip", "w", zipfile.ZIP_DEFLATED) as z:
            for p in srcs:
                z.write(p, p.name)
        note("photos.zip", kind="archive")
        if (OUT / "deck.pptx").exists():
            with zipfile.ZipFile(OUT / "with_deck.zip", "w", zipfile.ZIP_STORED) as z:
                z.write(OUT / "deck.pptx", "deck.pptx")
                z.write(srcs[0], srcs[0].name)
            note("with_deck.zip", kind="archive")
    folder = OUT / "Grandkids"
    folder.mkdir(exist_ok=True)
    for i in range(23):
        photo_like(3000, 2000, 300 + i).save(folder / f"IMG_{1000 + i}.jpg", quality=94)
    note("Grandkids", kind="folder", files=23)

if want("text"):
    print("text")
    rnd = random.Random(5)
    words = ["".join(rnd.choices("abcdefghijklmnopqrstuvwxyz", k=rnd.randint(2, 9))) for _ in range(5000)]
    with open(OUT / "big_log.txt", "w") as f:
        while f.tell() < 30_000_000:
            f.write(" ".join(rnd.choices(words, k=20)) + "\n")
    note("big_log.txt", kind="text")
    with open(OUT / "random.bin", "wb") as f:
        f.write(os.urandom(25_000_000))
    note("random.bin", kind="other")

(OUT / "manifest.json").write_text(json.dumps(manifest, indent=2, ensure_ascii=False))
print(f"wrote {len(manifest)} fixtures to {OUT}")
