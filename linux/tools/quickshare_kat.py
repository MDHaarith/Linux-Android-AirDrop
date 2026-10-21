# Independent reference for the UKEY2/Nearby Share key derivation (RFC 5869 HKDF by hand),
# following NearDrop's PROTOCOL.md and the google/ukey2 README.
import hashlib, hmac

def hkdf(ikm, salt, info, length=32):
    prk = hmac.new(salt, ikm, hashlib.sha256).digest()
    okm, t, i = b"", b"", 0
    while len(okm) < length:
        i += 1
        t = hmac.new(prk, t + info + bytes([i]), hashlib.sha256).digest()
        okm += t
    return okm[:length]

def pin(auth):
    h, m = 0, 1
    for b in auth:
        s = b - 256 if b > 127 else b
        h = h + s * m
        # C/Java remainder truncates toward zero
        h = int(h - 9973 * int(h / 9973)) if h >= 0 else -((-h) % 9973)
        m = (m * 31) % 9973
    return "%04d" % abs(h)

dhs = hashlib.sha256(bytes([7]) * 32).digest()
info = b"client-init" + b"server-init"
auth = hkdf(dhs, b"UKEY2 v1 auth", info)
nxt = hkdf(dhs, b"UKEY2 v1 next", info)
d2d = hashlib.sha256(b"D2D").digest()
client = hkdf(nxt, d2d, b"client")
server = hkdf(nxt, d2d, b"server")
sm = hashlib.sha256(b"SecureMessage").digest()
print("AUTH", auth.hex())
print("NEXT", nxt.hex())
print("CLIENT_ENC", hkdf(client, sm, b"ENC:2").hex())
print("SERVER_SIG", hkdf(server, sm, b"SIG:1").hex())
print("PIN", pin(auth))
print("PIN[1,255]", pin(bytes([1, 255])))
