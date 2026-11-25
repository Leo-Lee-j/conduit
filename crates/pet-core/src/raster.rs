use crate::sprite::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i16,
    pub y: i16,
    pub w: i16,
    pub h: i16,
    pub color: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub rects: Vec<Rect>,
    cells: [[u8; CANVAS_W]; CANVAS_H],
}
impl Frame {
    pub fn to_ascii(&self) -> String {
        self.cells
            .iter()
            .map(|row| String::from_utf8_lossy(row).into_owned())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub fn rasterize(pose: &Pose, palette: &Palette) -> Frame {
    let mut cells = [[b'.'; CANVAS_W]; CANVAS_H];
    let shadow = if pose.hop == 0 { 4..14 } else { 5..13 };
    for x in shadow {
        cells[18][x] = b's';
    }
    let ox = 1 + i16::from(pose.shake.clamp(-1, 1));
    let oy = 6 - i16::from(pose.bob.min(1)) - i16::from(pose.hop.min(2));
    for (y, row) in BODY.iter().enumerate() {
        for (x, cell) in row.bytes().enumerate() {
            if cell != b'.' {
                cells[oy as usize + y][ox as usize + x] = cell;
            }
        }
    }
    // X eyes occupy cheek cells; this combination is normalized, not overpainted.
    if pose.cheeks && pose.eyes != EyeKind::X {
        for (x, y) in CHEEKS {
            cells[(oy + y) as usize][(ox + x) as usize] = b'C';
        }
    }
    for (x, y) in pose.eyes.cells(pose.look) {
        cells[(oy + y) as usize][(ox + x) as usize] = b'E';
    }
    for &(x, y) in pose.mouth.cells() {
        cells[(oy + y) as usize][(ox + x) as usize] = b'M';
    }
    let (points, ink) = fx_cells(pose.fx);
    for &(x, y) in &points {
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            cells[(y + dy) as usize][(x + dx) as usize] = b'w';
        }
    }
    for (x, y) in points {
        cells[y as usize][x as usize] = ink;
    }
    let color = |cell| match cell {
        b'o' | b'd' | b'z' => palette.outline,
        b'#' => palette.body,
        b'h' => palette.highlight,
        b'E' | b'M' => palette.ink,
        b'C' => palette.cheek,
        b's' => palette.shadow,
        b'w' => palette.halo,
        b'H' => palette.heart,
        b'!' => palette.amber,
        b'X' => palette.red,
        _ => 0,
    };
    let mut rects = Vec::new();
    for (y, row) in cells.iter().enumerate() {
        let mut x = 0;
        while x < CANVAS_W {
            if row[x] == b'.' {
                x += 1;
                continue;
            }
            let start = x;
            let rgba = color(row[x]);
            x += 1;
            while x < CANVAS_W && row[x] != b'.' && color(row[x]) == rgba {
                x += 1;
            }
            rects.push(Rect {
                x: start as i16,
                y: y as i16,
                w: (x - start) as i16,
                h: 1,
                color: rgba,
            });
        }
    }
    Frame { rects, cells }
}

fn fx_cells(fx: Fx) -> (Vec<(i16, i16)>, u8) {
    let (bitmap, x, y, ink): (&[&str], i16, i16, u8) = match fx {
        Fx::None => return (vec![], b'.'),
        Fx::Dots(n) => {
            return (
                (0..n.min(3)).map(|i| (19 + i16::from(i) * 2, 5)).collect(),
                b'd',
            );
        }
        Fx::Exclaim { error } => (
            &[".#.", ".#.", ".#.", "...", ".#."],
            20,
            3,
            if error { b'X' } else { b'!' },
        ),
        Fx::Heart(rise) => (
            &[".#.#.", "#####", "#####", ".###.", "..#.."],
            19,
            6 - i16::from(rise.min(3)),
            b'H',
        ),
        Fx::Zzz(step) => (
            &["###", "..#", ".#.", "#..", "###"],
            20,
            6 - i16::from(step.min(3)),
            b'z',
        ),
    };
    let mut cells = vec![];
    for (dy, row) in bitmap.iter().enumerate() {
        for (dx, pixel) in row.bytes().enumerate() {
            if pixel == b'#' {
                cells.push((x + dx as i16, y + dy as i16));
            }
        }
    }
    (cells, ink)
}
