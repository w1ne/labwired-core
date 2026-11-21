// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The finite roots of `det(A0 + s·A1) = 0` for real square `A0`, `A1`: the
//! `.pz` root finder.
//!
//! With a real shift `σ` that leaves `M = A0 + σ·A1` regular,
//! `A0 + s·A1 = M·(I + (s − σ)·M⁻¹·A1)`, so each nonzero eigenvalue `μ` of
//! `M⁻¹·A1` gives the root `s = σ − 1/μ` and each zero eigenvalue a root at
//! infinity. A column of `A1` that is zero (a node or branch no capacitance
//! or inductance touches) gives an exactly zero column of `M⁻¹·A1`; such
//! columns, and any row or column with nothing off the diagonal, are taken
//! out before the QR iteration, so the roots at infinity they carry never
//! reach it. The rest is balanced, reduced to Hessenberg form and solved by
//! the shifted double-step QR iteration of EISPACK's `balanc`, `elmhes` and
//! `hqr`.

/// Why no roots could be found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PencilError {
    /// `A0 + σ·A1` was singular at every trial shift: the determinant is
    /// zero for every `s`.
    Singular,
    /// The QR iteration did not converge.
    NoConvergence,
}

/// The finite roots `(re, im)` of `det(A0 + s·A1)`, both row-major `n × n`.
/// A complex pair is returned as both of its members.
pub(crate) fn pencil_roots(
    a0: &[f64],
    a1: &[f64],
    n: usize,
) -> Result<Vec<(f64, f64)>, PencilError> {
    let norm = |m: &[f64]| m.iter().map(|v| v * v).sum::<f64>().sqrt();
    let (norm0, norm1) = (norm(a0), norm(a1));
    if n == 0 || norm1 == 0.0 {
        return Ok(Vec::new());
    }
    // `σ = 0` first; a pencil singular there (a root at s = 0) takes a shift
    // on the scale of its own roots instead. The factors are arbitrary and
    // irrational so that a shift lands on a root only by accident.
    let scale = if norm0 > 0.0 { norm0 / norm1 } else { 1.0 };
    for factor in [
        0.0,
        0.618_033_988_749_894_8,
        -1.324_717_957_244_746,
        2.236_067_977_499_79,
    ] {
        let sigma = factor * scale;
        let shifted: Vec<f64> = a0.iter().zip(a1).map(|(x, y)| x + sigma * y).collect();
        let Some(mut a) = solve_columns(shifted, a1.to_vec(), n) else {
            continue;
        };
        let eigenvalues = eigenvalues(&mut a, n)?;
        let size = eigenvalues
            .iter()
            .map(|&(re, im)| re.hypot(im))
            .fold(0.0, f64::max);
        // An eigenvalue that is zero but for rounding is a root at infinity.
        let tolerance = 1e-13 * size;
        return Ok(eigenvalues
            .into_iter()
            .filter(|&(re, im)| re.hypot(im) > tolerance)
            .map(|(re, im)| {
                let magnitude = re * re + im * im;
                (sigma - re / magnitude, im / magnitude)
            })
            .collect());
    }
    Err(PencilError::Singular)
}

/// `M⁻¹·B` by Gaussian elimination with partial pivoting, after scaling each
/// row of `M` and `B` to a largest `|M|` entry of one (which leaves `M⁻¹·B`
/// unchanged). `None` when a pivot is below `1e-14`.
fn solve_columns(mut m: Vec<f64>, mut b: Vec<f64>, n: usize) -> Option<Vec<f64>> {
    for row in 0..n {
        let largest = m[row * n..(row + 1) * n]
            .iter()
            .fold(0.0_f64, |acc, v| acc.max(v.abs()));
        if largest == 0.0 {
            return None;
        }
        for column in 0..n {
            m[row * n + column] /= largest;
            b[row * n + column] /= largest;
        }
    }
    for column in 0..n {
        let pivot_row = (column..n)
            .max_by(|&x, &y| m[x * n + column].abs().total_cmp(&m[y * n + column].abs()))
            .expect("non-empty range");
        if m[pivot_row * n + column].abs() < 1e-14 {
            return None;
        }
        if pivot_row != column {
            for index in 0..n {
                m.swap(pivot_row * n + index, column * n + index);
                b.swap(pivot_row * n + index, column * n + index);
            }
        }
        let pivot = m[column * n + column];
        for row in (column + 1)..n {
            let factor = m[row * n + column] / pivot;
            if factor == 0.0 {
                continue;
            }
            for index in column..n {
                m[row * n + index] -= factor * m[column * n + index];
            }
            for index in 0..n {
                b[row * n + index] -= factor * b[column * n + index];
            }
        }
    }
    for row in (0..n).rev() {
        for index in 0..n {
            let tail: f64 = ((row + 1)..n)
                .map(|k| m[row * n + k] * b[k * n + index])
                .sum();
            b[row * n + index] = (b[row * n + index] - tail) / m[row * n + row];
        }
    }
    Some(b)
}

/// Every eigenvalue `(re, im)` of the row-major `n × n` matrix `a`, which is
/// overwritten.
fn eigenvalues(a: &mut [f64], n: usize) -> Result<Vec<(f64, f64)>, PencilError> {
    // Take out each index whose row or column has nothing off the diagonal:
    // the matrix is block-triangular about it, with the diagonal entry as an
    // eigenvalue.
    let mut values = Vec::new();
    let mut live: Vec<usize> = (0..n).collect();
    loop {
        let isolated = live.iter().position(|&j| {
            let column = live.iter().all(|&i| i == j || a[i * n + j] == 0.0);
            let row = live.iter().all(|&i| i == j || a[j * n + i] == 0.0);
            column || row
        });
        match isolated {
            Some(at) => {
                let j = live.remove(at);
                values.push((a[j * n + j], 0.0));
            }
            None => break,
        }
    }
    let k = live.len();
    if k == 0 {
        return Ok(values);
    }
    // 1-based, as EISPACK writes it.
    let mut h = vec![0.0; (k + 1) * (k + 1)];
    for (r, &i) in live.iter().enumerate() {
        for (c, &j) in live.iter().enumerate() {
            h[(r + 1) * (k + 1) + c + 1] = a[i * n + j];
        }
    }
    let mut m = Matrix { data: h, n: k };
    m.balance();
    m.hessenberg();
    values.extend(m.hqr()?);
    Ok(values)
}

/// A 1-based square matrix, so the EISPACK loops read as written.
struct Matrix {
    data: Vec<f64>,
    n: usize,
}

impl Matrix {
    fn at(&self, i: usize, j: usize) -> f64 {
        self.data[i * (self.n + 1) + j]
    }

    fn at_mut(&mut self, i: usize, j: usize) -> &mut f64 {
        &mut self.data[i * (self.n + 1) + j]
    }

    /// EISPACK `balanc`: scale rows and columns by powers of two until their
    /// off-diagonal norms match. A similarity, so the eigenvalues are kept.
    fn balance(&mut self) {
        const RADIX: f64 = 2.0;
        let n = self.n;
        let mut done = false;
        while !done {
            done = true;
            for i in 1..=n {
                let (mut c, mut r): (f64, f64) = (0.0, 0.0);
                for j in 1..=n {
                    if j != i {
                        c += self.at(j, i).abs();
                        r += self.at(i, j).abs();
                    }
                }
                if c == 0.0 || r == 0.0 {
                    continue;
                }
                let s = c + r;
                let mut f = 1.0;
                let mut g = r / RADIX;
                while c < g {
                    f *= RADIX;
                    c *= RADIX * RADIX;
                }
                g = r * RADIX;
                while c > g {
                    f /= RADIX;
                    c /= RADIX * RADIX;
                }
                if (c + r) / f < 0.95 * s {
                    done = false;
                    for j in 1..=n {
                        *self.at_mut(i, j) /= f;
                        *self.at_mut(j, i) *= f;
                    }
                }
            }
        }
    }

    /// EISPACK `elmhes`: reduce to upper Hessenberg form by stabilised
    /// elementary similarities, then clear what lies below the subdiagonal.
    fn hessenberg(&mut self) {
        let n = self.n;
        for m in 2..n {
            let mut x: f64 = 0.0;
            let mut i = m;
            for j in m..=n {
                if self.at(j, m - 1).abs() > x.abs() {
                    x = self.at(j, m - 1);
                    i = j;
                }
            }
            if i != m {
                for j in (m - 1)..=n {
                    let (a, b) = (self.at(i, j), self.at(m, j));
                    *self.at_mut(i, j) = b;
                    *self.at_mut(m, j) = a;
                }
                for j in 1..=n {
                    let (a, b) = (self.at(j, i), self.at(j, m));
                    *self.at_mut(j, i) = b;
                    *self.at_mut(j, m) = a;
                }
            }
            if x != 0.0 {
                for i in (m + 1)..=n {
                    let mut y = self.at(i, m - 1);
                    if y != 0.0 {
                        y /= x;
                        *self.at_mut(i, m - 1) = y;
                        for j in m..=n {
                            let v = self.at(m, j);
                            *self.at_mut(i, j) -= y * v;
                        }
                        for j in 1..=n {
                            let v = self.at(j, i);
                            *self.at_mut(j, m) += y * v;
                        }
                    }
                }
            }
        }
        for i in 3..=n {
            for j in 1..(i - 1) {
                *self.at_mut(i, j) = 0.0;
            }
        }
    }

    /// EISPACK `hqr`: the eigenvalues of an upper Hessenberg matrix by the
    /// Francis double-shift QR iteration, with an exceptional shift every
    /// 10th iteration on one eigenvalue.
    fn hqr(&mut self) -> Result<Vec<(f64, f64)>, PencilError> {
        const MAX_ITERATIONS: usize = 60;
        let n = self.n;
        let mut wr = vec![0.0; n + 1];
        let mut wi = vec![0.0; n + 1];
        let mut anorm = 0.0;
        for i in 1..=n {
            for j in i.saturating_sub(1).max(1)..=n {
                anorm += self.at(i, j).abs();
            }
        }
        let mut nn = n;
        let mut t = 0.0;
        while nn >= 1 {
            let mut its = 0;
            loop {
                // The lowest `l` whose subdiagonal entry is negligible.
                let mut l = nn;
                while l >= 2 {
                    let mut s = self.at(l - 1, l - 1).abs() + self.at(l, l).abs();
                    if s == 0.0 {
                        s = anorm;
                    }
                    if self.at(l, l - 1).abs() + s == s {
                        *self.at_mut(l, l - 1) = 0.0;
                        break;
                    }
                    l -= 1;
                }
                let mut x = self.at(nn, nn);
                if l == nn {
                    wr[nn] = x + t;
                    wi[nn] = 0.0;
                    nn -= 1;
                    break;
                }
                let mut y = self.at(nn - 1, nn - 1);
                let mut w = self.at(nn, nn - 1) * self.at(nn - 1, nn);
                if l == nn - 1 {
                    let p = 0.5 * (y - x);
                    let q = p * p + w;
                    let mut z = q.abs().sqrt();
                    x += t;
                    if q >= 0.0 {
                        z = p + z.copysign(p);
                        wr[nn - 1] = x + z;
                        wr[nn] = if z != 0.0 { x - w / z } else { x + z };
                        wi[nn - 1] = 0.0;
                        wi[nn] = 0.0;
                    } else {
                        wr[nn - 1] = x + p;
                        wr[nn] = x + p;
                        wi[nn - 1] = -z;
                        wi[nn] = z;
                    }
                    nn = nn.saturating_sub(2);
                    break;
                }
                if its == MAX_ITERATIONS {
                    return Err(PencilError::NoConvergence);
                }
                if its > 0 && its % 10 == 0 {
                    t += x;
                    for i in 1..=nn {
                        *self.at_mut(i, i) -= x;
                    }
                    let s = self.at(nn, nn - 1).abs() + self.at(nn - 1, nn - 2).abs();
                    x = 0.75 * s;
                    y = x;
                    w = -0.4375 * s * s;
                }
                its += 1;
                // Look for two consecutive small subdiagonal entries.
                let mut m = nn - 2;
                let (mut p, mut q, mut r);
                loop {
                    let z = self.at(m, m);
                    let rr = x - z;
                    let ss = y - z;
                    p = (rr * ss - w) / self.at(m + 1, m) + self.at(m, m + 1);
                    q = self.at(m + 1, m + 1) - z - rr - ss;
                    r = self.at(m + 2, m + 1);
                    let s = p.abs() + q.abs() + r.abs();
                    p /= s;
                    q /= s;
                    r /= s;
                    if m == l {
                        break;
                    }
                    let u = self.at(m, m - 1).abs() * (q.abs() + r.abs());
                    let v = p.abs()
                        * (self.at(m - 1, m - 1).abs() + z.abs() + self.at(m + 1, m + 1).abs());
                    if u + v == v {
                        break;
                    }
                    m -= 1;
                }
                for i in (m + 2)..=nn {
                    *self.at_mut(i, i - 2) = 0.0;
                    if i != m + 2 {
                        *self.at_mut(i, i - 3) = 0.0;
                    }
                }
                // Double QR step on rows `l..=nn`, columns `m..=nn`.
                for k in m..nn {
                    if k != m {
                        p = self.at(k, k - 1);
                        q = self.at(k + 1, k - 1);
                        r = if k != nn - 1 {
                            self.at(k + 2, k - 1)
                        } else {
                            0.0
                        };
                        x = p.abs() + q.abs() + r.abs();
                        if x != 0.0 {
                            p /= x;
                            q /= x;
                            r /= x;
                        }
                    }
                    let s = (p * p + q * q + r * r).sqrt().copysign(p);
                    if s == 0.0 {
                        continue;
                    }
                    if k == m {
                        if l != m {
                            *self.at_mut(k, k - 1) = -self.at(k, k - 1);
                        }
                    } else {
                        *self.at_mut(k, k - 1) = -s * x;
                    }
                    p += s;
                    x = p / s;
                    y = q / s;
                    let z = r / s;
                    q /= p;
                    r /= p;
                    for j in k..=nn {
                        let mut pp = self.at(k, j) + q * self.at(k + 1, j);
                        if k != nn - 1 {
                            pp += r * self.at(k + 2, j);
                            *self.at_mut(k + 2, j) -= pp * z;
                        }
                        *self.at_mut(k + 1, j) -= pp * y;
                        *self.at_mut(k, j) -= pp * x;
                    }
                    let last = nn.min(k + 3);
                    for i in l..=last {
                        let mut pp = x * self.at(i, k) + y * self.at(i, k + 1);
                        if k != nn - 1 {
                            pp += z * self.at(i, k + 2);
                            *self.at_mut(i, k + 2) -= pp * r;
                        }
                        *self.at_mut(i, k + 1) -= pp * q;
                        *self.at_mut(i, k) -= pp;
                    }
                }
            }
        }
        Ok((1..=n).map(|i| (wr[i], wi[i])).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(mut roots: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
        roots.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        roots
    }

    #[test]
    fn companion_matrix_roots() {
        // det(A0 + s·I) = 0 for A0 = −companion(s³ − 2s² − 5s + 6 =
        // (s − 1)(s + 2)(s − 3)): s = −2, 1, 3.
        let a0 = [-2.0, -5.0, 6.0, -1.0, 0.0, 0.0, 0.0, -1.0, 0.0];
        let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        let roots = sorted(pencil_roots(&a0, &identity, 3).unwrap());
        for ((re, im), expected) in roots.iter().zip([-2.0, 1.0, 3.0]) {
            assert!(
                (re - expected).abs() < 1e-12 && im.abs() < 1e-12,
                "{roots:?}"
            );
        }
    }

    #[test]
    fn complex_pair_and_root_at_infinity() {
        // [[s + 1, −2], [2, 1]]: det = s + 1 + 4 → s = −5, one root only.
        let roots = pencil_roots(&[1.0, -2.0, 2.0, 1.0], &[1.0, 0.0, 0.0, 0.0], 2).unwrap();
        assert_eq!(roots.len(), 1);
        assert!((roots[0].0 + 5.0).abs() < 1e-12 && roots[0].1 == 0.0);
        // s² + 2s + 5 = det [[s + 2, 5], [−1, s]]: s = −1 ± 2j.
        let roots = sorted(pencil_roots(&[2.0, 5.0, -1.0, 0.0], &[1.0, 0.0, 0.0, 1.0], 2).unwrap());
        assert!((roots[0].0 + 1.0).abs() < 1e-12 && (roots[0].1 + 2.0).abs() < 1e-12);
        assert!((roots[1].0 + 1.0).abs() < 1e-12 && (roots[1].1 - 2.0).abs() < 1e-12);
    }

    #[test]
    fn root_at_zero_takes_a_shift_and_singular_pencil_is_an_error() {
        // diag(s, s + 3): A0 singular at σ = 0.
        let roots = sorted(pencil_roots(&[0.0, 0.0, 0.0, 3.0], &[1.0, 0.0, 0.0, 1.0], 2).unwrap());
        assert!(
            (roots[0].0 + 3.0).abs() < 1e-12 && roots[1].0.abs() < 1e-12,
            "{roots:?}"
        );
        // A zero row: singular for every s.
        assert_eq!(
            pencil_roots(&[1.0, 0.0, 0.0, 0.0], &[1.0, 0.0, 0.0, 0.0], 2),
            Err(PencilError::Singular)
        );
    }

    #[test]
    fn hessenberg_qr_on_a_dense_matrix() {
        // Eigenvalues of a full 5×5 with a known spectrum, `Q·diag·Q⁻¹` for
        // a full `Q`: the roots of det(−A + s·I) are the diagonal.
        let n = 5;
        let diagonal = [-4.0, -1.5, 0.5, 2.0, 7.0];
        let identity: Vec<f64> = (0..n * n)
            .map(|k| if k % (n + 1) == 0 { 1.0 } else { 0.0 })
            .collect();
        let q: Vec<f64> = (0..n * n)
            .map(|k| identity[k] + 0.4 * libm::sin(1.0 + 3.7 * k as f64))
            .collect();
        let q_inverse = solve_columns(q.clone(), identity.clone(), n).unwrap();
        let mut a = vec![0.0; n * n];
        for i in 0..n {
            for j in 0..n {
                a[i * n + j] = -(0..n)
                    .map(|k| q[i * n + k] * diagonal[k] * q_inverse[k * n + j])
                    .sum::<f64>();
            }
        }
        let roots = sorted(pencil_roots(&a, &identity, n).unwrap());
        for ((re, im), expected) in roots.iter().zip(diagonal) {
            assert!((re - expected).abs() < 1e-9 && im.abs() < 1e-9, "{roots:?}");
        }
    }
}
