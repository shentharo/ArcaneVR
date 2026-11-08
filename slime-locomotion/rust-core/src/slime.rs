//! Bounded SlimeVR packet parsing; mirrors the existing Python receiver.
use crate::gait::{unit, Q};
/// Optional MAC from a tracker discovery body. Legacy or malformed bodies carry no identity.
pub fn discovery_mac(body: &[u8]) -> Option<String> {
    let len = *body.get(28)? as usize;
    let start = 29usize.checked_add(len)?;
    let mac: [u8; 6] = body.get(start..start.checked_add(6)?)?.try_into().ok()?;
    if mac == [0; 6] || mac[0] & 1 != 0 { return None; }
    Some(mac.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":"))
}
pub fn unpack(data: &[u8]) -> Result<(u64, Vec<(u32, &[u8])>), &'static str> {
    if data.len() < 12 {
        return Err("short header");
    }
    let kind = u32::from_be_bytes(data[..4].try_into().unwrap());
    let seq = u64::from_be_bytes(data[4..12].try_into().unwrap());
    let mut body = &data[12..];
    if kind != 100 && kind != 101 {
        return Ok((seq, vec![(kind, body)]));
    }
    let mut out = vec![];
    let (ns, ids) = if kind == 100 { (2, 4) } else { (1, 1) };
    while !body.is_empty() {
        if out.len() >= 256 || body.len() < ns {
            return Err("invalid bundle");
        }
        let n = if ns == 2 {
            u16::from_be_bytes(body[..2].try_into().unwrap()) as usize
        } else {
            body[0] as usize
        };
        body = &body[ns..];
        if n == 0 {
            continue;
        }
        if n < ids || n > body.len() {
            return Err("short bundle");
        }
        let part = &body[..n];
        body = &body[n..];
        let k = if ids == 4 {
            u32::from_be_bytes(part[..4].try_into().unwrap())
        } else {
            part[0] as u32
        };
        out.push((k, &part[ids..]));
    }
    Ok((seq, out))
}
pub fn rotation(kind: u32, b: &[u8]) -> Result<Option<(u8, Q)>, &'static str> {
    let (sensor, offset) = match kind {
        1 => (0, 0),
        16 => (1, 0),
        17 => {
            if b.len() < 19 {
                return Err("short rotation");
            }
            if b[1] != 1 {
                return Ok(None);
            }
            (b[0], 2)
        }
        23 => {
            if b.len() < 15 {
                return Err("short compact rotation");
            }
            let mut q = [0.; 4];
            for i in 0..4 {
                q[i] =
                    i16::from_be_bytes(b[1 + i * 2..3 + i * 2].try_into().unwrap()) as f64 / 32768.;
            }
            return Ok(Some((b[0], unit([q[3], q[0], q[1], q[2]])?)));
        }
        _ => return Ok(None),
    };
    if b.len() < offset + 16 {
        return Err("short rotation");
    }
    let mut q = [0.; 4];
    for i in 0..4 {
        q[i] = f32::from_be_bytes(b[offset + i * 4..offset + i * 4 + 4].try_into().unwrap()) as f64;
    }
    Ok(Some((sensor, unit([q[3], q[0], q[1], q[2]])?)))
}
/// SlimeVR linear acceleration: float32 XYZ, optional legacy sensor ID; or Q7 XYZ in packet 23.
pub fn acceleration(kind: u32, b: &[u8]) -> Result<Option<(u8, [f64; 3])>, &'static str> {
    let (id, a) = match kind {
        4 => {
            if b.len() < 12 {
                return Err("short acceleration");
            }
            let mut a = [0.; 3];
            for i in 0..3 {
                a[i] = f32::from_be_bytes(b[i * 4..i * 4 + 4].try_into().unwrap()) as f64;
            }
            (b.get(12).copied().unwrap_or(0), a)
        }
        23 => {
            if b.len() < 15 {
                return Err("short compact acceleration");
            }
            let mut a = [0.; 3];
            for i in 0..3 {
                a[i] =
                    i16::from_be_bytes(b[9 + i * 2..11 + i * 2].try_into().unwrap()) as f64 / 128.;
            }
            (b[0], a)
        }
        _ => return Ok(None),
    };
    if !a.iter().all(|x| x.is_finite()) {
        return Err("invalid acceleration");
    }
    Ok(Some((id, a)))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wire_and_bundles() {
        let mut q = vec![0, 1];
        for v in [0f32, 0., 0., 1.] {
            q.extend(v.to_be_bytes());
        }
        q.push(3);
        assert_eq!(rotation(17, &q), Ok(Some((0, [1., 0., 0., 0.]))));
        for kind in [100u32, 101] {
            let mut p = kind.to_be_bytes().to_vec();
            p.extend(42u64.to_be_bytes());
            let mut inner = if kind == 100 {
                17u32.to_be_bytes().to_vec()
            } else {
                vec![17]
            };
            inner.extend(&q);
            if kind == 100 {
                p.extend((inner.len() as u16).to_be_bytes());
            } else {
                p.push(inner.len() as u8);
            }
            p.extend(inner);
            let (_, packets) = unpack(&p).unwrap();
            assert_eq!(rotation(packets[0].0, packets[0].1).unwrap().unwrap().0, 0);
        }
    }
    #[test]
    fn compact_rotation_requires_acceleration_payload() {
        let mut b = vec![0; 15];
        b[7] = 0x7f;
        b[8] = 0xff;
        assert!(rotation(23, &b[..9]).is_err());
        assert!(rotation(23, &b).unwrap().is_some());
    }
    #[test]
    fn acceleration_wire_formats_and_invalid_values() {
        let mut b = Vec::new();
        for value in [1.5f32, -2., 0.25] {
            b.extend(value.to_be_bytes());
        }
        b.push(7);
        assert_eq!(acceleration(4, &b).unwrap(), Some((7, [1.5, -2., 0.25])));
        assert!(acceleration(4, &b[..11]).is_err());
        b[..4].copy_from_slice(&f32::NAN.to_be_bytes());
        assert!(acceleration(4, &b).is_err());
        let mut compact = vec![0; 15];
        compact[0] = 3;
        compact[9..11].copy_from_slice(&192i16.to_be_bytes());
        compact[11..13].copy_from_slice(&(-256i16).to_be_bytes());
        compact[13..15].copy_from_slice(&32i16.to_be_bytes());
        assert_eq!(
            acceleration(23, &compact).unwrap(),
            Some((3, [1.5, -2., 0.25]))
        );
        assert!(acceleration(23, &compact[..14]).is_err());
    }
    #[test]
    fn malformed_corpus() {
        for n in 0..2048 {
            let b = vec![(n % 256) as u8; n];
            let _ = unpack(&b);
            let _ = discovery_mac(&b);
            for k in [1, 16, 17, 23] {
                let _ = rotation(k, &b);
            }
        }
    }
    #[test]
    fn discovery_identity_is_bounded_and_unicast() {
        let mut b=vec![0;28]; b.push(3);b.extend(b"0.7");b.extend([0x1c,0x8f,0x57,0x82,0x6c,0x63,1]);
        assert_eq!(discovery_mac(&b).as_deref(),Some("1C:8F:57:82:6C:63"));
        for n in 0..38 { assert!(discovery_mac(&b[..n]).is_none()); }
        b[32]=1; assert!(discovery_mac(&b).is_none());
    }
}
