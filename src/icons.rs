use ksni::Icon;

pub const SIZE: usize = 22;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub fn luminance(self) -> f64 {
        fn ch(v: u8) -> f64 {
            let c = v as f64 / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * ch(self.r) + 0.7152 * ch(self.g) + 0.0722 * ch(self.b)
    }

    pub fn contrast(self, other: Rgb) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }
}

/// ARGB32 network byte order — так его ждёт StatusNotifierItem.
fn fg(color: Rgb, alpha: u8) -> [u8; 4] {
    [alpha, color.r, color.g, color.b]
}

/// Чашка бодрящего напитка — инхибитор включён.
pub fn coffee(color: Rgb) -> Icon {
    let mut px = [[0u8; 4]; SIZE * SIZE];
    let solid = fg(color, 0xFF);
    let steam = fg(color, 0xB4);

    for y in 9..17usize {
        let (x0, x1) = if y < 15 { (3, 14) } else { (4, 13) };
        for x in x0..=x1 {
            if x == x0 || x == x1 || y == 9 || y == 16 {
                px[y * SIZE + x] = solid;
            }
        }
    }
    for x in 4..14 {
        px[10 * SIZE + x] = fg(color, 0x90);
    }
    for x in 15..18 {
        px[11 * SIZE + x] = solid;
        px[13 * SIZE + x] = solid;
    }
    px[12 * SIZE + 18] = solid;
    px[12 * SIZE + 19] = solid;
    for x in 1..18 {
        px[18 * SIZE + x] = solid;
        px[19 * SIZE + x] = solid;
    }
    for (x, y0, y1) in [(6usize, 2usize, 7usize), (9, 1, 7), (12, 2, 7)] {
        for y in y0..y1 {
            px[y * SIZE + x] = steam;
        }
        px[(y1 - 1) * SIZE + (x + 1)] = steam;
    }
    to_icon(px)
}

/// Луна и «Zzz» — сон и блокировка разрешены.
pub fn moon(color: Rgb) -> Icon {
    let mut px = [[0u8; 4]; SIZE * SIZE];
    let solid = fg(color, 0xFF);
    let dim = fg(color, 0x90);

    // полумесяц: диск минус смещённый диск
    let (cx, cy, r) = (11.0f32, 11.0f32, 8.0f32);
    let (ox, oy) = (14.5f32, 9.0f32);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let in_disc = dx * dx + dy * dy <= r * r;
            let ex = x as f32 + 0.5 - ox;
            let ey = y as f32 + 0.5 - oy;
            let in_cut = ex * ex + ey * ey <= r * r;
            if in_disc && !in_cut {
                px[y * SIZE + x] = solid;
            }
        }
    }
    // маленькие «z» рядом — намёк на сон
    draw_z(&mut px, 15, 1, 4, dim);
    draw_z(&mut px, 17, 10, 3, dim);
    to_icon(px)
}

fn draw_z(px: &mut [[u8; 4]; SIZE * SIZE], x0: usize, y0: usize, s: usize, color: [u8; 4]) {
    let y1 = y0 + s - 1;
    let x1 = x0 + s - 1;
    if x1 >= SIZE || y1 >= SIZE {
        return;
    }
    for x in x0..=x1 {
        px[y0 * SIZE + x] = color;
        px[y1 * SIZE + x] = color;
    }
    let steps = s.saturating_sub(1) as i32;
    let mut last_x = x1 as i32;
    for i in 0..=steps {
        let y = y0 as i32 + i;
        let x = x1 as i32 - (i * (x1 as i32 - x0 as i32)) / steps.max(1);
        for xx in x..=last_x {
            px[y as usize * SIZE + xx as usize] = color;
        }
        last_x = x;
    }
}

fn to_icon(px: [[u8; 4]; SIZE * SIZE]) -> Icon {
    Icon {
        width: SIZE as i32,
        height: SIZE as i32,
        data: px.into_iter().flatten().collect(),
    }
}

pub fn png(icon: &Icon) -> Vec<u8> {
    let w = icon.width as u32;
    let h = icon.height as u32;
    let mut raw = Vec::with_capacity((w * h * 4 + h) as usize);
    for y in 0..h {
        raw.push(0u8);
        let start = (y * w * 4) as usize;
        for px in icon.data[start..start + (w * 4) as usize].chunks_exact(4) {
            raw.extend_from_slice(&[px[1], px[2], px[3], px[0]]);
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    out.extend_from_slice(&chunk(b"IHDR", &ihdr));
    out.extend_from_slice(&chunk(b"IDAT", &zlib(&raw)));
    out.extend_from_slice(&chunk(b"IEND", &[]));
    out
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut c = Vec::with_capacity(data.len() + 12);
    c.extend_from_slice(&(data.len() as u32).to_be_bytes());
    c.extend_from_slice(kind);
    c.extend_from_slice(data);
    let mut crc_in = kind.to_vec();
    crc_in.extend_from_slice(data);
    c.extend_from_slice(&crc32(&crc_in).to_be_bytes());
    c
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut i = 0;
    while i < data.len() {
        let len = (data.len() - i).min(65535);
        let block = &data[i..i + len];
        let last = if i + len == data.len() { 1u8 } else { 0 };
        out.push(last);
        out.extend_from_slice(&(len as u16).to_le_bytes());
        out.extend_from_slice(&(!(len as u16)).to_le_bytes());
        out.extend_from_slice(block);
        i += len;
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_are_argb() {
        let coffee = coffee(Rgb { r: 0, g: 0, b: 0 });
        let visible: Vec<_> = coffee.data.chunks(4).filter(|p| p[0] > 0).collect();
        assert!(!visible.is_empty(), "кофе должен иметь видимые пиксели");
        assert!(visible.iter().all(|p| p[1] == 0 && p[2] == 0 && p[3] == 0));

        let moon = moon(Rgb {
            r: 255,
            g: 255,
            b: 255,
        });
        let visible: Vec<_> = moon.data.chunks(4).filter(|p| p[0] > 0).collect();
        assert!(visible.len() > 50, "луна слишком пустая: {}", visible.len());
        assert!(visible
            .iter()
            .all(|p| p[1] == 255 && p[2] == 255 && p[3] == 255));
    }
}
