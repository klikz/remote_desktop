"""Build Ulan icon assets from the source logo (assets/logo/source.png)."""
import numpy as np, os, sys
from PIL import Image, ImageDraw
here = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'assets', 'logo')
src = Image.open(os.path.join(here, 'source.png')).convert('RGB')
a = np.asarray(src).astype(float)
minc = a.min(axis=2)
alpha = np.clip((248 - minc) / 60.0, 0, 1)
safe = np.maximum(alpha, 1e-3)[..., None]
rgb = np.clip((a - 255 * (1 - alpha[..., None])) / safe, 0, 255)
out = np.dstack([rgb, alpha * 255]).astype(np.uint8)
img = Image.fromarray(out, 'RGBA')
img = img.crop(img.getchannel('A').point(lambda v: 255 if v > 8 else 0).getbbox())
S = 1024; pad = int(S * 0.04)
w, h = img.size; k = (S - 2 * pad) / max(w, h)
img = img.resize((round(w * k), round(h * k)), Image.LANCZOS)
logo = Image.new('RGBA', (S, S), (0, 0, 0, 0))
logo.paste(img, ((S - img.width) // 2, (S - img.height) // 2), img)
logo.save(os.path.join(here, 'ulan-logo.png'))

def badge(size, bg=(255, 255, 255, 255), radius=0.22, scale=0.72):
    b = Image.new('RGBA', (size, size), (0, 0, 0, 0))
    ImageDraw.Draw(b).rounded_rectangle([0, 0, size - 1, size - 1], int(size * radius), fill=bg)
    m = logo.resize((int(size * scale),) * 2, Image.LANCZOS)
    b.paste(m, ((size - m.width) // 2,) * 2, m)
    return b

badge(512).save(os.path.join(here, 'ulan-icon-512.png'))
for s in (16, 20, 24, 32, 40, 48, 64, 128, 256):
    logo.resize((s, s), Image.LANCZOS).save(os.path.join(here, f'ulan-{s}.png'))
badge(256).save(os.path.join(here, 'app.ico'), sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)])
# Android: adaptive foreground (logo within 66% safe zone) + legacy launcher pngs
res = os.path.join(here, '..', '..', 'mobile_viewer', 'app', 'src', 'main', 'res')
for d, px in (('mdpi', 48), ('hdpi', 72), ('xhdpi', 96), ('xxhdpi', 144), ('xxxhdpi', 192)):
    p = os.path.join(res, f'mipmap-{d}'); os.makedirs(p, exist_ok=True)
    badge(px).save(os.path.join(p, 'ic_launcher.png'))
    badge(px, radius=0.5).save(os.path.join(p, 'ic_launcher_round.png'))
fg = Image.new('RGBA', (432, 432), (0, 0, 0, 0))
m = logo.resize((250, 250), Image.LANCZOS); fg.paste(m, (91, 91), m)
dr = os.path.join(res, 'drawable-nodpi'); os.makedirs(dr, exist_ok=True)
fg.save(os.path.join(dr, 'ic_launcher_foreground.png'))
print('ok')
