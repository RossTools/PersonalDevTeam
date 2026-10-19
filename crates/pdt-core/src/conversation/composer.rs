//! What the conversation composer offers beyond text (structured agents spec §5, §7): `@` file suggestions and image
//! attachments. Pure.

/// The largest image a message may carry (10 MB).
pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
/// The most images one message may carry.
pub const MAX_IMAGES: usize = 10;

/// The image type of `bytes` by their first bytes (never the name or the sender's say-so): PNG, JPEG, GIF or WebP.
pub fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// The file extension for an image type from [`image_type`].
pub fn image_extension(media_type: &str) -> &'static str {
    match media_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        _ => "webp",
    }
}

/// A plausible attachment id (a UUID as PDT makes them): what `…/attachments/{id}` and a message's `images` accept.
pub fn valid_attachment_id(id: &str) -> bool {
    id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// How well `query` matches `path` (higher is better), or None. Case-insensitive. Best: the file name starts with the
/// query; then the name contains it; then the path contains it; then the query's characters appear in order.
fn score(query: &str, path: &str) -> Option<i64> {
    let q = query.to_lowercase();
    let p = path.to_lowercase();
    let name = p.rsplit('/').next().unwrap_or(&p);
    // Shorter paths win ties.
    let short = -(p.len() as i64);
    if q.is_empty() {
        return Some(short);
    }
    if name.starts_with(&q) {
        return Some(4_000 + short);
    }
    if name.contains(&q) {
        return Some(3_000 + short);
    }
    if p.contains(&q) {
        return Some(2_000 + short);
    }
    let mut rest = p.chars();
    q.chars().all(|c| rest.any(|x| x == c)).then_some(1_000 + short)
}

/// The best `limit` paths for an `@` query, best first.
pub fn rank_paths<'a>(query: &str, paths: &'a [String], limit: usize) -> Vec<&'a str> {
    let mut scored: Vec<(i64, &str)> = paths.iter().filter_map(|p| score(query, p).map(|s| (s, p.as_str()))).collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored.into_iter().take(limit).map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_are_known_by_their_bytes() {
        assert_eq!(image_type(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(image_type(b"\xff\xd8\xff\xe0"), Some("image/jpeg"));
        assert_eq!(image_type(b"GIF89a.."), Some("image/gif"));
        assert_eq!(image_type(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(image_type(b"<svg xmlns"), None, "SVG is not accepted");
        assert_eq!(image_type(b""), None);
        assert_eq!(image_extension("image/jpeg"), "jpg");
        assert!(valid_attachment_id("01a1546a-24ca-72e6-8206-3ae44a03d42d"));
        assert!(!valid_attachment_id("../../etc/passwd"));
    }

    #[test]
    fn paths_rank_by_name_then_path_then_letters() {
        let paths: Vec<String> = [
            "web/src/conversation/MessageList.tsx",
            "web/ui/Message.tsx",
            "docs/messages.md",
            "crates/pdt-api/src/sessions.rs",
            "web/src/mobile/ess.ts",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            rank_paths("mess", &paths, 10),
            ["docs/messages.md", "web/ui/Message.tsx", "web/src/conversation/MessageList.tsx", "web/src/mobile/ess.ts"]
        );
        assert_eq!(rank_paths("conv/msg", &paths, 10), ["web/src/conversation/MessageList.tsx"]);
        assert_eq!(rank_paths("", &paths, 2).len(), 2);
        assert!(rank_paths("zzz", &paths, 10).is_empty());
    }
}
