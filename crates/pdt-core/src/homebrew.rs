//! Bumping a Homebrew cask to a new release (PDT-373): its version from the release's tag, the macOS file to take,
//! and the cask's text with the new `version` and `sha256`.

/// A cask's file in its tap, by Homebrew's layout (`Casks/<name>.rb`).
pub fn cask_path(cask: &str) -> String {
    format!("Casks/{}.rb", cask.trim())
}

/// A cask's version from a release tag: `v1.2.0` is `1.2.0`; a tag that isn't `v` and a digit stays as it is.
pub fn cask_version(tag: &str) -> String {
    let tag = tag.trim();
    match tag.strip_prefix(['v', 'V']) {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest.to_string(),
        _ => tag.to_string(),
    }
}

/// The macOS file among a release's `names`: the first matching `pattern` (a glob, `*` any text), or without one
/// the first `.dmg`, then `.app.tar.gz`, then `.app.zip`, then a `.zip` naming mac or darwin.
pub fn mac_asset<'a>(names: &[&'a str], pattern: Option<&str>) -> Result<Option<&'a str>, String> {
    if let Some(p) = pattern.map(str::trim).filter(|p| !p.is_empty()) {
        let m = crate::overlap::glob(p)?.compile_matcher();
        return Ok(names.iter().copied().find(|n| m.is_match(n)));
    }
    let lower = |n: &str| n.to_ascii_lowercase();
    let tests: [&dyn Fn(&str) -> bool; 4] = [
        &|n| lower(n).ends_with(".dmg"),
        &|n| lower(n).ends_with(".app.tar.gz"),
        &|n| lower(n).ends_with(".app.zip"),
        &|n| {
            let l = lower(n);
            l.ends_with(".zip") && (l.contains("mac") || l.contains("darwin"))
        },
    ];
    Ok(tests.iter().find_map(|t| names.iter().copied().find(|n| t(n))))
}

/// A cask's text bumped to `version` and `sha256`: its `version "…"` and `sha256 "…"` lines, and the old version
/// wherever its `url` spells it out (a url with `#{version}` needs nothing). Unchanged when it is already there.
pub fn bump_cask(text: &str, version: &str, sha256: &str) -> Result<String, String> {
    let quoted = |line: &str, key: &str| -> Option<String> {
        let rest = line.trim_start().strip_prefix(key)?.strip_prefix(char::is_whitespace)?.trim_start();
        let inner = rest.strip_prefix('"')?;
        inner.find('"').map(|end| inner[..end].to_string())
    };
    let mut old = None;
    let (mut versions, mut sums) = (0, 0);
    for line in text.lines() {
        let t = line.trim_start();
        if let Some(v) = quoted(line, "version") {
            versions += 1;
            old.get_or_insert(v);
        } else if t.starts_with("version ") {
            return Err(format!("the cask's version isn't one quoted string: {}", t.trim_end()));
        }
        if quoted(line, "sha256").is_some() {
            sums += 1;
        } else if t.starts_with("sha256 ") {
            return Err(format!(
                "the cask has a checksum per CPU or none ({}): it takes one file for every Mac",
                t.trim_end()
            ));
        }
    }
    let old = old.ok_or("the cask has no version \"…\" line")?;
    if versions > 1 || sums != 1 {
        return Err(format!("the cask has {versions} version and {sums} sha256 lines: it takes one of each"));
    }
    // The line with its (first) quoted string replaced.
    let set = |line: &str, to: &str| {
        let start = line.find('"').unwrap_or(line.len());
        let end = line[start + 1..].find('"').map_or(line.len(), |e| start + 1 + e + 1);
        format!("{}\"{to}\"{}", &line[..start], &line[end..])
    };
    let mut out: Vec<String> = vec![];
    for line in text.split_inclusive('\n') {
        let (body, nl) = match line.strip_suffix('\n') {
            Some(b) => (b, "\n"),
            None => (line, ""),
        };
        let new = if quoted(body, "version").is_some() {
            set(body, version)
        } else if quoted(body, "sha256").is_some() {
            set(body, sha256)
        } else if body.trim_start().starts_with("url ") && !old.is_empty() && old != version {
            body.replace(&old, version)
        } else {
            body.to_string()
        };
        out.push(format!("{new}{nl}"));
    }
    Ok(out.concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CASK: &str = r#"cask "thing" do
  version "0.1.0"
  sha256 "aaaa"

  url "https://github.com/o/thing/releases/download/v#{version}/Thing.dmg"
  name "Thing"
  app "Thing.app"
end
"#;

    #[test]
    fn a_tag_gives_the_version() {
        assert_eq!(cask_version("v1.2.0"), "1.2.0");
        assert_eq!(cask_version(" V2.0 "), "2.0");
        assert_eq!(cask_version("1.2.0"), "1.2.0");
        assert_eq!(cask_version("version-3"), "version-3", "a v not before a digit stays");
        assert_eq!(cask_path("thing"), "Casks/thing.rb");
    }

    #[test]
    fn the_mac_file_is_a_dmg_then_an_app_archive_or_the_pattern() {
        let names = ["thing_1.0_amd64.deb", "Thing.app.tar.gz", "thing-setup.exe", "Thing_1.0.dmg", "SHA256SUMS"];
        assert_eq!(mac_asset(&names, None).unwrap(), Some("Thing_1.0.dmg"));
        assert_eq!(mac_asset(&names[..3], None).unwrap(), Some("Thing.app.tar.gz"));
        assert_eq!(mac_asset(&["t-darwin.zip", "t-win.zip"], None).unwrap(), Some("t-darwin.zip"));
        assert_eq!(mac_asset(&["t.exe"], None).unwrap(), None);
        assert_eq!(mac_asset(&names, Some("*.app.tar.gz")).unwrap(), Some("Thing.app.tar.gz"));
        assert_eq!(mac_asset(&names, Some(" ")).unwrap(), Some("Thing_1.0.dmg"), "a blank pattern is none");
        assert_eq!(mac_asset(&names, Some("*.pkg")).unwrap(), None);
    }

    #[test]
    fn a_cask_takes_the_new_version_and_checksum() {
        let got = bump_cask(CASK, "0.2.0", "bbbb").unwrap();
        assert_eq!(got, CASK.replace("\"0.1.0\"", "\"0.2.0\"").replace("\"aaaa\"", "\"bbbb\""));
        assert_eq!(bump_cask(&got, "0.2.0", "bbbb").unwrap(), got, "already there: unchanged");
    }

    #[test]
    fn a_url_that_spells_out_the_version_takes_the_new_one() {
        let cask = "cask \"t\" do\n  version \"0.1.0\"\n  sha256 \"aa\"\n  url \"https://x/0.1.0/T-0.1.0.dmg\"\nend";
        assert_eq!(
            bump_cask(cask, "0.2.0", "bb").unwrap(),
            "cask \"t\" do\n  version \"0.2.0\"\n  sha256 \"bb\"\n  url \"https://x/0.2.0/T-0.2.0.dmg\"\nend"
        );
    }

    #[test]
    fn a_cask_it_cant_bump_says_why() {
        let per_cpu = CASK.replace("sha256 \"aaaa\"", "sha256 arm:   \"aa\",\n         intel: \"bb\"");
        assert!(bump_cask(&per_cpu, "1", "x").unwrap_err().contains("per CPU"));
        let no_check = CASK.replace("sha256 \"aaaa\"", "sha256 :no_check");
        assert!(bump_cask(&no_check, "1", "x").unwrap_err().contains("per CPU or none"));
        assert!(bump_cask("cask \"t\" do\nend\n", "1", "x").unwrap_err().contains("no version"));
        let latest = CASK.replace("version \"0.1.0\"", "version :latest");
        assert!(bump_cask(&latest, "1", "x").unwrap_err().contains("quoted"));
    }
}
