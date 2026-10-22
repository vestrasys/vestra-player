//! A Win32 `.res` file (the output of `rc.exe`) for vestra-host.exe, built in
//! Rust so no resource compiler is needed: the application icon
//! (`windows/vestra.ico`, from `src/packaging/icon.mjs`) and VERSIONINFO.
//! `build.rs` passes the file to link.exe, which embeds it (cvtres).
//! Layout: Microsoft "Resource File Formats", `VS_VERSIONINFO`.

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
// Memory flags as rc.exe writes them: MOVEABLE | DISCARDABLE for icon
// images, plus PURE for the icon group, MOVEABLE | PURE for the version.
const ICON_FLAGS: u16 = 0x1010;
const GROUP_FLAGS: u16 = 0x1030;
const VERSION_FLAGS: u16 = 0x0030;
const LANGUAGE_EN_US: u16 = 0x0409;

/// `version` is `major.minor.patch[-pre]`; strings are product metadata.
pub fn resource_file(
    ico: &[u8],
    version: &str,
    strings: &[(&str, &str)],
) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    // Every .res file starts with an empty entry (type 0, name 0).
    entry(&mut out, 0, 0, 0, &[]);
    let images = icon_images(ico)?;
    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(images.len() as u16).to_le_bytes());
    for (index, image) in images.iter().enumerate() {
        let id = index as u16 + 1;
        entry(&mut out, RT_ICON, id, ICON_FLAGS, image.data);
        // GRPICONDIRENTRY: the ICONDIRENTRY fields, with a resource id in place of the offset.
        group.extend_from_slice(&image.header[..12]);
        group.extend_from_slice(&id.to_le_bytes());
    }
    entry(&mut out, RT_GROUP_ICON, 1, GROUP_FLAGS, &group);
    entry(
        &mut out,
        RT_VERSION,
        1,
        VERSION_FLAGS,
        &version_info(version, strings),
    );
    Ok(out)
}

struct IconImage<'a> {
    header: &'a [u8],
    data: &'a [u8],
}

fn icon_images(ico: &[u8]) -> Result<Vec<IconImage<'_>>, String> {
    let u16_at = |at: usize| {
        ico.get(at..at + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
    };
    let u32_at = |at: usize| {
        ico.get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    if u16_at(0) != Some(0) || u16_at(2) != Some(1) {
        return Err("not an .ico file".into());
    }
    let count = u16_at(4).ok_or("truncated .ico")? as usize;
    (0..count)
        .map(|index| {
            let at = 6 + index * 16;
            let header = ico.get(at..at + 16).ok_or("truncated .ico directory")?;
            let size = u32_at(at + 8).ok_or("truncated .ico directory")?;
            let offset = u32_at(at + 12).ok_or("truncated .ico directory")?;
            let data = ico
                .get(offset..offset + size)
                .ok_or("truncated .ico image")?;
            Ok(IconImage { header, data })
        })
        .collect()
}

/// One resource: header (numeric type and name), then the data, DWORD aligned.
fn entry(out: &mut Vec<u8>, kind: u16, name: u16, flags: u16, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes());
    out.extend_from_slice(&[0xFF, 0xFF]);
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&[0xFF, 0xFF]);
    out.extend_from_slice(&name.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&(if kind == 0 { 0 } else { LANGUAGE_EN_US }).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // Version
    out.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
    out.extend_from_slice(data);
    pad(out);
}

fn pad(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn utf16z(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// A version-resource block: wLength, wValueLength, wType, szKey, value, children.
/// `value_length` is in bytes for binary values and in WCHARs for text.
fn block(key: &str, value: &[u8], value_length: u16, text: bool, children: &[Vec<u8>]) -> Vec<u8> {
    let mut out = vec![0, 0];
    out.extend_from_slice(&value_length.to_le_bytes());
    out.extend_from_slice(&u16::from(text).to_le_bytes());
    out.extend_from_slice(&utf16z(key));
    pad(&mut out);
    out.extend_from_slice(value);
    for child in children {
        pad(&mut out);
        out.extend_from_slice(child);
    }
    let length = out.len() as u16;
    out[..2].copy_from_slice(&length.to_le_bytes());
    out
}

fn version_info(version: &str, strings: &[(&str, &str)]) -> Vec<u8> {
    let mut parts = version
        .split(['-', '+'])
        .next()
        .unwrap_or("0")
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0));
    let (major, minor, patch) = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );
    let ms = (major << 16) | (minor & 0xFFFF);
    let ls = patch << 16;
    let mut fixed = Vec::new();
    for value in [
        0xFEEF_04BD_u32, // dwSignature
        0x0001_0000,     // structure version 1.0
        ms,
        ls, // file version
        ms,
        ls,          // product version
        0x3F,        // dwFileFlagsMask
        0,           // dwFileFlags
        0x0004_0004, // VOS_NT_WINDOWS32
        1,           // VFT_APP
        0,
        0,
        0,
    ] {
        fixed.extend_from_slice(&value.to_le_bytes());
    }
    let texts: Vec<Vec<u8>> = strings
        .iter()
        .map(|(key, value)| {
            let bytes = utf16z(value);
            block(key, &bytes, (bytes.len() / 2) as u16, true, &[])
        })
        .collect();
    let table = block("040904B0", &[], 0, true, &texts);
    let string_info = block("StringFileInfo", &[], 0, true, &[table]);
    let translation = [0x09, 0x04, 0xB0, 0x04];
    let var = block("Translation", &translation, 4, false, &[]);
    let var_info = block("VarFileInfo", &[], 0, true, &[var]);
    block(
        "VS_VERSION_INFO",
        &fixed,
        fixed.len() as u16,
        false,
        &[string_info, var_info],
    )
}
