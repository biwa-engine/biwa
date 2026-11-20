"""例のゲームの音の素材を合成する (標準ライブラリだけ。CC0 として同梱する)。"""
import math, random, struct, sys, wave, os

RATE = 22050
out = sys.argv[1]

def write(path, samples, channels=1, peak=None):
    if peak is not None:
        m = max(abs(x) for x in samples) or 1.0
        samples = [x * peak / m for x in samples]
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with wave.open(path, "wb") as w:
        w.setnchannels(channels); w.setsampwidth(2); w.setframerate(RATE)
        w.writeframes(b"".join(struct.pack("<h", max(-32767, min(32767, int(s * 32767)))) for s in samples))

def env(i, n, attack, release):
    a = min(1.0, i / max(1, attack)); r = min(1.0, (n - i) / max(1, release)); return min(a, r)

def note(freq, dur, amp=0.2, decay=3.0, harmonics=(1.0, 0.4, 0.2)):
    n = int(RATE * dur)
    return [amp * math.exp(-decay * i / RATE) * env(i, n, 200, 400) *
            sum(h * math.sin(2 * math.pi * freq * (k + 1) * i / RATE) for k, h in enumerate(harmonics))
            for i in range(n)]

def mix(track, part, at):
    start = int(at * RATE)
    for i, s in enumerate(part):
        if start + i < len(track): track[start + i] += s

def midi(m): return 440 * 2 ** ((m - 69) / 12)

# BGM: 8 秒でループする穏やかなアルペジオ (C - Am - F - G)。
def bgm(chords, length, tempo, amp):
    track = [0.0] * int(RATE * length)
    beat = 60 / tempo
    t = 0.0
    for chord in chords:
        for k in range(4):
            mix(track, note(midi(chord[k % 3] + 12 * (k // 3)), beat * 2, amp, 1.5), t + k * beat)
        mix(track, note(midi(chord[0] - 12), beat * 4, amp * 0.8, 0.6, (1.0, 0.1)), t)
        t += beat * 4
    return track

write(f"{out}/sounds/bgm/morning.wav", bgm([(60, 64, 67), (57, 60, 64), (53, 57, 60), (55, 59, 62)], 8.0, 120, 0.12))
write(f"{out}/sounds/bgm/evening.wav", bgm([(57, 60, 64), (53, 57, 60), (50, 53, 57), (52, 56, 59)], 8.0, 120, 0.11))

# SE: チャイムと足音。
chime = note(midi(84), 0.9, 0.35, 4.0, (1.0, 0.0, 0.3, 0.0, 0.1))
mix(chime, note(midi(91), 0.8, 0.25, 4.0, (1.0, 0.0, 0.3)), 0.12)
write(f"{out}/sounds/se/chime.wav", chime)
random.seed(1)
step = []
for k in range(2):
    n = int(RATE * 0.09); prev = 0.0
    for i in range(n):
        prev = prev * 0.6 + random.uniform(-1, 1) * 0.4
        step.append(prev * 0.5 * math.exp(-40 * i / RATE))
    step += [0.0] * int(RATE * 0.22)
write(f"{out}/sounds/se/steps.wav", step)

# 環境音: 4 秒でループする風 (低くこもったノイズ。端は同じ値に寄せてつなぎ目を消す)。
n = int(RATE * 4.0); wind = []; lp = 0.0
for i in range(n):
    lp = lp * 0.995 + random.uniform(-1, 1) * 0.005
    swell = 0.6 + 0.4 * math.sin(2 * math.pi * i / n)
    wind.append(lp * 6.0 * swell)
fade = int(RATE * 0.2)
for i in range(fade):
    k = i / fade; wind[i] *= k; wind[n - 1 - i] *= k
write(f"{out}/sounds/ambient/wind.wav", wind, peak=0.35)

# ボイス: 母音らしい音 (のこぎり波に 2 つのフォルマントの共振を掛ける)。言葉ではない。
def vowel_line(syllables, pitch):
    track = []
    for f1, f2, dur in syllables:
        n = int(RATE * dur); y1 = [0.0, 0.0]; y2 = [0.0, 0.0]
        def res(f, bw):
            r = math.exp(-math.pi * bw / RATE); c = 2 * r * math.cos(2 * math.pi * f / RATE); return c, -r * r
        c1, d1 = res(f1, 90); c2, d2 = res(f2, 120); phase = 0.0
        for i in range(n):
            vib = 1 + 0.02 * math.sin(2 * math.pi * 5 * i / RATE)
            phase = (phase + pitch * vib / RATE) % 1.0
            src = 2 * phase - 1
            a = src + c1 * y1[0] + d1 * y1[1]; y1 = [a, y1[0]]
            b = src + c2 * y2[0] + d2 * y2[1]; y2 = [b, y2[0]]
            track.append((a * 0.012 + b * 0.008) * env(i, n, 300, 600))
        track += [0.0] * int(RATE * 0.04)
    return track

A, I, U, E, O = (800, 1200), (300, 2300), (350, 1300), (500, 1900), (500, 800)
write(f"{out}/sounds/voice/biwa_001.wav", vowel_line([(*E, .14), (*O, .12), (*A, .16), (*I, .12), (*A, .22)], 260), peak=0.5)
write(f"{out}/sounds/voice/biwa_002.wav", vowel_line([(*U, .12), (*A, .18), (*I, .12), (*E, .14), (*O, .12), (*U, .24)], 280), peak=0.5)
print("ok")
