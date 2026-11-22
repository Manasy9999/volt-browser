# Génère l'icône de Volt (Volty statique de logos/v3) : volt.ico multi-tailles + volt.png + volty64.rgba (icône de fenêtre tao).
# Lancer depuis la racine du projet : uv run --with pillow python logos/make_icon.py
# Pillow seul (pas de cairo à installer) : mêmes formes que le SVG, dessinées en sur-échantillonnage.
from pathlib import Path
from PIL import Image, ImageDraw

C, EYE = (155, 92, 255, 255), (11, 12, 16, 255)  # violet Volt, yeux couleur du fond
BOLT = [(39, 0), (24, 15), (32, 15), (28, 25), (43, 10), (35, 10)]  # M39 0L24 15h8l-4 10 15-15h-8z
VIEW_X, VIEW_Y, VIEW = 2, -1, 60  # cadrage carré du viewBox 64x64 (éclair + corps)


def volty(size, k=8):
    n = size * k
    s = n / VIEW
    p = lambda x, y: ((x - VIEW_X) * s, (y - VIEW_Y) * s)
    im = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.polygon([p(x, y) for x, y in BOLT], fill=C)
    d.rounded_rectangle([p(8, 22), p(56, 58)], radius=14 * s, fill=C)  # corps
    for x in (17, 38):  # yeux « normal » : ovales 9 x 13
        d.rounded_rectangle([p(x, 33), p(x + 9, 46)], radius=4.5 * s, fill=EYE)
    return im.resize((size, size), Image.LANCZOS)


out = Path(__file__).parent
big = volty(256)
big.save(out / "volt.png")
big.save(out / "volt.ico", sizes=[(n, n) for n in (16, 24, 32, 48, 64, 256)], append_images=[volty(n) for n in (16, 24, 32, 48, 64)])
(out / "volty64.rgba").write_bytes(volty(64).tobytes())

# auto-vérification : toutes les tailles présentes dans le .ico, RGBA brut à la bonne taille
ico = Image.open(out / "volt.ico")
assert ico.info["sizes"] == {(n, n) for n in (16, 24, 32, 48, 64, 256)}, ico.info["sizes"]
assert len((out / "volty64.rgba").read_bytes()) == 64 * 64 * 4
print("ok", sorted(ico.info["sizes"]))
