"""Prints the 8-bit code OBS 33 writes for a gray scRGB pixel of L nits on an SDR
canvas, for its default Reinhard tonemap and the HDR Tone Mapping filter's MaxRGB
SDR, ported from libobs/data/color.effect and hdr_tonemap_filter.effect.

python3 tonemap.py"""
m1, m2, c1, c2, c3 = 0.1593017578125, 78.84375, 0.8359375, 18.8515625, 18.6875
def pq(y):  # linear (1.0 = 10000 nits) -> PQ
    y = max(y, 0) ** m1; return ((c1 + c2 * y) / (1 + c3 * y)) ** m2
def ipq(e):
    e = e ** (1 / m2); return (max(e - c1, 0) / (c2 - c3 * e)) ** (1 / m1)
def eetf(L, Lw, Lmax):  # nits -> nits, eetf_0_Lmax in color.effect
    Lwp = pq(Lw / 1e4); E1 = min(pq(L / 1e4) / Lwp, 1); ml = pq(Lmax / 1e4) / Lwp; KS = 1.5 * ml - 0.5
    E2 = E1
    if E1 > KS:
        T = (E1 - KS) / (1 - KS); T2, T3 = T * T, T * T * T
        E2 = (2*T3 - 3*T2 + 1) * KS + (T3 - 2*T2 + T) * (1 - KS) + (-2*T3 + 3*T2) * ml
    return ipq(E2 * Lwp) * 1e4
code = lambda x: round(255 * min(max(x, 0), 1) ** (1 / 2.4))
sdr_white = 80  # scRGB 1.0
levels = [5, 20, 50, 100, 150, 200, 300, 500, 1000]
print("tonemap".ljust(26), *[f"{L:>5}" for L in levels])
for S in (300, 200, 120):
    print(f"Reinhard white {S}".ljust(26), *[f"{code((L/S)/(1+L/S)):>5}" for L in levels])
for Lw in (1000,):
    for Lmax in (300, 200, 150, 120):
        print(f"MaxRGB SDR {Lw}->{Lmax}".ljust(26), *[f"{code(eetf(L, Lw, Lmax)/Lmax):>5}" for L in levels])
