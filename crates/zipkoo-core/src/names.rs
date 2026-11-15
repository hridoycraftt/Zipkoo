//! File names from old archives that predate UTF-8. ZIPs made by Windows tools in Japan, China,
//! Korea, Russia… store names in the local code page with no marker. The bytes are decoded with
//! a guess: UTF-8 if valid, else the most likely legacy encoding (chardetng), falling back to
//! the original IBM PC code page (CP437) that the ZIP standard names. Results are NFC-normalized
//! so names from macOS (decomposed accents) match everywhere else.
use unicode_normalization::UnicodeNormalization;

pub fn decode_legacy(raw: &[u8]) -> String {
    let s = if let Ok(s) = std::str::from_utf8(raw) {
        s.to_string()
    } else if raw.iter().all(|&b| b < 0x80) {
        String::from_utf8_lossy(raw).into_owned()
    } else {
        let mut det = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
        det.feed(raw, true);
        let enc = det.guess(None, chardetng::Utf8Detection::Deny);
        let (text, _, bad) = enc.decode(raw);
        if bad {
            cp437(raw)
        } else {
            text.into_owned()
        }
    };
    nfc(&s)
}

/// The name of a ZIP entry, decoded the same way everywhere (list, extract, edit, update).
pub fn zip_name<R: std::io::Read>(f: &zip::read::ZipFile<'_, R>) -> String {
    decode_legacy(f.name_raw())
}

pub fn nfc(s: &str) -> String {
    s.nfc().collect()
}

fn cp437(raw: &[u8]) -> String {
    const HIGH: &str = "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■\\u{a0}";
    let high: Vec<char> = HIGH.chars().collect();
    raw.iter()
        .map(|&b| {
            if b < 0x80 {
                b as char
            } else {
                high.get((b - 0x80) as usize).copied().unwrap_or('?')
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_common_legacy_names() {
        // Shift-JIS "資料.txt"
        let (sjis, _, _) = encoding_rs::SHIFT_JIS.encode("資料フォルダ/報告書.txt");
        assert_eq!(decode_legacy(&sjis), "資料フォルダ/報告書.txt");
        // GBK "文件.txt"
        let (gbk, _, _) = encoding_rs::GBK.encode("中文文件名称/报告.txt");
        assert_eq!(decode_legacy(&gbk), "中文文件名称/报告.txt");
        // UTF-8 stays as is, and NFD becomes NFC.
        assert_eq!(decode_legacy("cafe\u{301}.txt".as_bytes()), "café.txt");
        assert_eq!(decode_legacy(b"plain.txt"), "plain.txt");
    }
}
