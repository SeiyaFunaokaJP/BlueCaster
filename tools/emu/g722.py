"""G.722 decoder (64 kbit/s) for the hearing aid checks of run_test.py --asha.

The same steps as the decoder of app/src/crates/codec/src/g722.rs (a port
of the public-domain SpanDSP code): the lower band is rebuilt from the 4 top
bits of its code. Enough to tell which tone a hearing aid got.
"""

ILB = [2048, 2093, 2139, 2186, 2233, 2282, 2332, 2383, 2435, 2489, 2543, 2599, 2656, 2714, 2774, 2834,
       2896, 2960, 3025, 3091, 3158, 3228, 3298, 3371, 3444, 3520, 3597, 3676, 3756, 3838, 3922, 4008]
WL = [-60, -30, 58, 172, 334, 538, 1198, 3042]
RL42 = [0, 7, 6, 5, 4, 3, 2, 1, 7, 6, 5, 4, 3, 2, 1, 0]
QM4 = [0, -20456, -12896, -8968, -6288, -4240, -2584, -1200, 20456, 12896, 8968, 6288, 4240, 2584, 1200, 0]
QM2 = [-7408, -1616, 7408, 1616]
QMF = [3, -11, 12, 32, -210, 951, 3876, -805, 362, -156, 53, -11]
WH = [0, -214, 798]
RH2 = [2, 1, 2, 1]


def saturate(v):
    return max(-32768, min(32767, v))


class Band:
    def __init__(self, det):
        self.s = self.sp = self.sz = self.nb = 0
        self.det = det
        self.r = [0] * 3
        self.a = [0] * 3
        self.ap = [0] * 3
        self.p = [0] * 3
        self.d = [0] * 7
        self.b = [0] * 7
        self.bp = [0] * 7

    def scale(self, shift):
        wd1 = (self.nb >> 6) & 31
        wd2 = shift - (self.nb >> 11)
        wd3 = ILB[wd1] << -wd2 if wd2 < 0 else ILB[wd1] >> wd2
        self.det = wd3 << 2

    def block4(self, d):
        self.d[0] = d
        self.r[0] = saturate(self.s + d)
        self.p[0] = saturate(self.sz + d)
        sg = [self.p[i] >> 15 for i in range(3)]
        wd1 = saturate(self.a[1] << 2)
        wd2 = min(-wd1 if sg[0] == sg[1] else wd1, 32767)
        ap2 = (wd2 >> 7) + (128 if sg[0] == sg[2] else -128) + ((self.a[2] * 32512) >> 15)
        self.ap[2] = max(-12288, min(12288, ap2))
        wd1 = 192 if sg[0] == sg[1] else -192
        wd2 = (self.a[1] * 32640) >> 15
        wd3 = saturate(15360 - self.ap[2])
        self.ap[1] = max(-wd3, min(wd3, saturate(wd1 + wd2)))
        wd1 = 0 if d == 0 else 128
        sg0 = d >> 15
        for i in range(1, 7):
            wd2 = wd1 if self.d[i] >> 15 == sg0 else -wd1
            self.bp[i] = saturate(wd2 + ((self.b[i] * 32640) >> 15))
        sz = 0
        for i in range(6, 0, -1):
            self.d[i] = self.d[i - 1]
            self.b[i] = self.bp[i]
            sz += (self.b[i] * saturate(self.d[i] + self.d[i])) >> 15
        self.sz = sz
        for i in range(2, 0, -1):
            self.r[i] = self.r[i - 1]
            self.p[i] = self.p[i - 1]
            self.a[i] = self.ap[i]
        wd1 = (self.a[1] * saturate(self.r[1] + self.r[1])) >> 15
        wd2 = (self.a[2] * saturate(self.r[2] + self.r[2])) >> 15
        self.sp = saturate(wd1 + wd2)
        self.s = saturate(self.sp + self.sz)


def decode(data):
    """16 kHz samples (two per byte) of G.722 `data`"""
    low, high = Band(32), Band(8)
    x = [0] * 24
    pcm = []
    for code in data:
        ril, ihigh = (code >> 2) & 0x0F, code >> 6
        dlow = (low.det * QM4[ril]) >> 15
        rlow = max(-16384, min(16383, low.s + dlow))
        low.nb = max(0, min(18432, ((low.nb * 127) >> 7) + WL[RL42[ril]]))
        low.scale(8)
        low.block4(dlow)
        dhigh = (high.det * QM2[ihigh]) >> 15
        rhigh = max(-16384, min(16383, high.s + dhigh))
        high.nb = max(0, min(22528, ((high.nb * 127) >> 7) + WH[RH2[ihigh]]))
        high.scale(10)
        high.block4(dhigh)
        x = x[2:] + [rlow + rhigh, rlow - rhigh]
        xout1 = sum(x[2 * i + 1] * QMF[11 - i] for i in range(12))
        xout2 = sum(x[2 * i] * QMF[i] for i in range(12))
        pcm.append(saturate(xout1 >> 11))
        pcm.append(saturate(xout2 >> 11))
    return pcm
