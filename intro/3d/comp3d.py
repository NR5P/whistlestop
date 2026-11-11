"""Composite the Blender frames: rain, title cards, hit flash, logo end card.

  cd intro && python 3d/comp3d.py | ffmpeg ... (raw BGR0 1920x1080 @30)
  cd intro && python 3d/comp3d.py --still 25.0 out.png
"""
import math
import os
import sys

import cairo

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import video_mountain as vm  # noqa: E402  (logo, cards, rain helpers)

W, H, FPS = 1920, 1080, 30
T_HIT, DUR, W_CQ = vm.T_HIT, vm.DUR, vm.W_CQ
FRAMES = os.path.join(HERE, "frames")


def logo_layer(cr, t):
    if t < T_HIT - 0.2:
        return
    cr.set_source_rgba(*vm.NAVY_B, 0.6 * vm.smooth((t - T_HIT + 0.2) / 1.2))
    cr.paint()
    ring_u = vm.ease_out((t - T_HIT - 0.1) / 0.7)
    wave_u = vm.ease_out((t - T_HIT - 0.55) / 0.6)
    wl = vm.whistle_level(t) if t > W_CQ - 0.1 else 0
    vm.draw_mark(cr, *vm.LOGO_C, vm.LOGO_R, 1.0, ring_u, wave_u, wl)
    for w0, gmax in ((W_CQ, 1.0), (W_CQ + 0.9, 0.45)):
        if w0 <= t < w0 + 0.8:
            ph = ((t - w0) % 0.4) / 0.2
            hx, hy = vm.logo_pt(ph if ph < 1 else 2 - ph)
            g = cairo.RadialGradient(hx, hy, 0, hx, hy, 70)
            g.add_color_stop_rgba(0, 1, 0.97, 0.85, 0.9 * gmax)
            g.add_color_stop_rgba(1, 1, 0.85, 0.5, 0)
            cr.save()
            cr.set_operator(cairo.OPERATOR_ADD)
            cr.set_source(g)
            cr.arc(hx, hy, 70, 0, 2 * math.pi)
            cr.fill()
            cr.restore()
        for k in range(4):
            tau = t - (w0 + k * 0.2)
            if 0 <= tau < 1.2:
                cr.set_source_rgba(*vm.BRASS, 0.5 * gmax * (1 - tau / 1.2) ** 2)
                cr.set_line_width(4)
                cr.arc(vm.LOGO_C[0], vm.LOGO_C[1], vm.LOGO_R + 30 + tau * 260, 0, 2 * math.pi)
                cr.stroke()
    pos, total = vm.text_spaced(cr, "WHISTLESTOP", W / 2, 0, 132, 22)
    for i, (ch, xx) in enumerate(pos):
        u = vm.ease_out((t - (T_HIT + 0.55 + i * 0.045)) / 0.45)
        if u <= 0:
            continue
        cr.move_to(xx, 745 + 40 * (1 - u))
        cr.text_path(ch)
        g = cairo.LinearGradient(0, 650, 0, 760)
        g.add_color_stop_rgba(0, *vm.BRASS_HI, u)
        g.add_color_stop_rgba(0.55, *vm.BRASS, u)
        g.add_color_stop_rgba(1, 0.7, 0.42, 0.12, u)
        cr.set_source(g)
        cr.fill()
    ru = vm.ease_out((t - (T_HIT + 1.0)) / 0.8)
    if ru > 0:
        cr.set_source_rgba(*vm.BRASS, 0.6)
        cr.rectangle(W / 2 - total / 2 * ru, 790, total * ru, 2)
        cr.fill()
    vm.soft_line(cr, "HF keyboard chat  ·  files  ·  Roundtables", T_HIT + 1.3, 99, t, 862, 40)
    vm.soft_line(cr, "USING THE MERCURY MODEM", T_HIT + 1.8, 99, t, 930, 32, col=vm.BRASS, spacing=9, bold=True)
    vm.soft_line(cr, "The whistle calls CQ.", T_HIT + 2.4, 99, t, 984, 26, col=(0.72, 0.69, 0.63))


def frame(t, fi):
    surf = cairo.ImageSurface.create_from_png(os.path.join(FRAMES, f"f_{fi + 1:04d}.png"))
    cr = cairo.Context(surf)
    fl = vm.flash_level(t)
    vm.draw_rain(cr, t, fl)
    vm.soft_line(cr, "There's only one way down.", 1.6, vm.T0 + 0.1, t, 150, 44, col=(0.92, 0.9, 0.85))
    for text, b0, b1 in vm.CARDS:
        vm.card(cr, text, vm.BARS[b0], vm.BARS[b1] - 0.05 if b1 < len(vm.BARS) - 1 else T_HIT - 0.05, t)
    logo_layer(cr, t)
    if T_HIT <= t < T_HIT + 0.6:
        cr.set_operator(cairo.OPERATOR_ADD)
        cr.set_source_rgba(1, 0.9, 0.7, 0.6 * math.exp(-(t - T_HIT) * 10))
        cr.paint()
        cr.set_operator(cairo.OPERATOR_OVER)
    g = cairo.RadialGradient(W / 2, H / 2, H * 0.4, W / 2, H / 2, H * 1.05)
    g.add_color_stop_rgba(0, 0, 0, 0, 0)
    g.add_color_stop_rgba(1, 0, 0, 0, 0.5)
    cr.set_source(g)
    cr.paint()
    a = max(1 - vm.smooth(t / 0.4), vm.smooth((t - (DUR - 1.4)) / 1.3))
    if a > 0:
        cr.set_source_rgba(0, 0, 0, a)
        cr.paint()
    surf.flush()
    return surf


if __name__ == "__main__":
    n = int(round(DUR * FPS))
    if len(sys.argv) > 2 and sys.argv[1] == "--still":
        t = float(sys.argv[2])
        frame(t, min(n - 1, int(t * FPS))).write_to_png(sys.argv[3])
    else:
        out = sys.stdout.buffer
        for i in range(n):
            out.write(bytes(frame(i / FPS, i).get_data()))
