"""Renders KTerm's icon and installer art. Run from anywhere: python installer/make_art.py
Outputs assets/kterm.ico (embedded in kterm.exe) and installer/art/*.png (used by kterm.iss)."""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent.parent
ART = ROOT / "installer" / "art"
FONT = str(ROOT / "assets" / "fonts" / "JetBrainsMonoNerdFont-Bold.ttf")
FONT_REG = str(ROOT / "assets" / "fonts" / "JetBrainsMonoNerdFont-Regular.ttf")

RIM, TILE, INK = (229, 229, 229), (48, 48, 48), (245, 245, 245)


def icon(px):
    """kitty-style tile: light rim, dark face, a small round-capped K and cursor bottom-left.
    Geometry is snapped to whole pixels at the final size, then drawn supersampled."""
    ss = 8
    pad = round(px * 0.06)
    rim = max(1, round(px * 0.045))
    h = max(7, round(px * 0.17))  # K height: small on big icons, the 7px floor keeps 16-32px legible
    small = h < 10
    w = 1 if small else round(h * 0.22)  # stroke; thicker than 1px merges the K into a blob below ~48px
    kw = round(h * (0.75 if small else 0.62))
    m = rim + max(2, round((px - 2 * pad) * 0.16))
    x0, y1 = pad + m, px - pad - m

    img = Image.new("RGBA", (px * ss, px * ss), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    box = lambda i: tuple(v * ss for v in (pad + i, pad + i, px - pad - i, px - pad - i))
    radius = (px - 2 * pad) * 0.23 * ss
    d.rounded_rectangle(box(0), radius, fill=RIM)
    d.rounded_rectangle(box(rim), radius - rim * ss, fill=TILE)

    def stroke(a, b):
        a, b = [(x * ss, y * ss) for x, y in (a, b)]
        d.line((a, b), fill=INK, width=w * ss)
        for x, y in (a, b):
            d.ellipse((x - w * ss / 2, y - w * ss / 2, x + w * ss / 2, y + w * ss / 2), fill=INK)

    c = w / 2  # round caps reach half a stroke past each endpoint
    stem_x, mid = x0 + c, y1 - h / 2
    stroke((stem_x, y1 - h + c), (stem_x, y1 - c))
    stroke((stem_x, mid), (x0 + kw - c, y1 - h + c))
    stroke((stem_x, mid), (x0 + kw - c, y1 - c))
    ux = x0 + kw + max(1, round(h * 0.3))
    stroke((ux + c, y1 - c), (ux + round(h * 0.55) - c, y1 - c))
    return img.resize((px, px), Image.LANCZOS)


def save_scaled(name, base_w, base_h, render, scales=(1, 1.25, 1.5, 1.75, 2, 2.25, 2.5)):
    """Inno picks the closest file per DPI from a comma-separated list."""
    names = []
    for sc in scales:
        fn = f"{name}-{round(sc * 100)}.png"
        render(round(base_w * sc), round(base_h * sc)).save(ART / fn)
        names.append(fn)
    return names


def header(w, h):
    """Header icon: Inno draws it flush with the corner, so give it breathing room."""
    img = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    ic = icon(round(w * 0.8))
    img.alpha_composite(ic, ((w - ic.width) // 2, (h - ic.height) // 2))
    return img


def welcome_panel(w, h):
    """Left panel of the welcome / finish pages: mark and wordmark on transparent."""
    img = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    ic = icon(round(w * 0.6))
    top = round(h * 0.4 - ic.height / 2)
    img.alpha_composite(ic, ((w - ic.width) // 2, top))
    d = ImageDraw.Draw(img)
    d.text((w / 2, top + ic.height * 1.18), "KTerm", font=ImageFont.truetype(FONT, round(w * 0.17)), fill=INK, anchor="mt")
    return img


if __name__ == "__main__":
    ART.mkdir(exist_ok=True)
    for old in ART.glob("*.png"):
        old.unlink()
    sizes = [16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 128, 256]
    frames = [icon(px) for px in sizes]
    frames[-1].save(ROOT / "assets" / "kterm.ico", sizes=[(p, p) for p in sizes], append_images=frames[:-1])
    icon(1024).save(ART / "kterm-1024.png")
    print("WizardSmallImageFile=" + ",".join(save_scaled("small", 55, 55, header)))
    print("WizardImageFile=" + ",".join(save_scaled("welcome", 164, 314, welcome_panel)))
