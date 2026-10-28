//! File-type classification used for colouring and the "File types" breakdown.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
#[repr(u8)]
pub enum Category {
    #[default]
    Other,
    Video,
    Image,
    Audio,
    Archive,
    Document,
    Code,
    Application,
    Game,
    System,
}

pub const COUNT: usize = 10;

pub const ALL: [Category; COUNT] = [
    Category::Other,
    Category::Video,
    Category::Image,
    Category::Audio,
    Category::Archive,
    Category::Document,
    Category::Code,
    Category::Application,
    Category::Game,
    Category::System,
];

impl Category {
    pub fn name(self) -> &'static str {
        match self {
            Category::Other => "Other",
            Category::Video => "Videos",
            Category::Image => "Images",
            Category::Audio => "Audio",
            Category::Archive => "Archives & disk images",
            Category::Document => "Documents",
            Category::Code => "Code & config",
            Category::Application => "Apps & libraries",
            Category::Game => "Game data",
            Category::System => "System & data",
        }
    }
}

/// Extension without the dot, or "" if the name has none (or an implausibly long one).
pub fn ext_of(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() && name.len() - i <= 13 => &name[i + 1..],
        _ => "",
    }
}

pub fn classify(name: &str) -> Category {
    if name.starts_with('$') {
        return Category::System; // NTFS metafiles such as $MFT
    }
    let ext = ext_of(name);
    if ext.is_empty() || ext.len() > 16 {
        return Category::Other;
    }
    let mut buf = [0u8; 16];
    let lower = &mut buf[..ext.len()];
    lower.copy_from_slice(ext.as_bytes());
    lower.make_ascii_lowercase();
    let e = std::str::from_utf8(lower).unwrap_or("");

    if e == "sys"
        && (name.eq_ignore_ascii_case("pagefile.sys")
            || name.eq_ignore_ascii_case("hiberfil.sys")
            || name.eq_ignore_ascii_case("swapfile.sys"))
    {
        return Category::System;
    }

    match e {
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "m4v" | "mpg" | "mpeg" | "ts"
        | "m2ts" | "mts" | "3gp" | "vob" | "ogv" => Category::Video,
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tif" | "tiff" | "webp" | "heic" | "heif"
        | "raw" | "cr2" | "cr3" | "nef" | "arw" | "dng" | "psd" | "svg" | "ico" | "tga" | "dds"
        | "exr" | "hdr" | "avif" | "jxl" => Category::Image,
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" | "wma" | "opus" | "aiff" | "aif"
        | "alac" | "mid" | "midi" => Category::Audio,
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "cab" | "iso"
        | "img" | "vhd" | "vhdx" | "vmdk" | "wim" | "esd" | "dmg" | "apk" => Category::Archive,
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "rtf" | "odt"
        | "ods" | "odp" | "csv" | "md" | "epub" | "xps" | "one" | "pst" | "ost" => {
            Category::Document
        }
        "cs" | "cpp" | "cc" | "c" | "h" | "hpp" | "js" | "mjs" | "tsx" | "jsx" | "py" | "java"
        | "go" | "rs" | "json" | "xml" | "yaml" | "yml" | "html" | "htm" | "css" | "scss"
        | "php" | "rb" | "kt" | "swift" | "sql" | "sh" | "ps1" | "bat" | "cmd" | "lua" | "ini"
        | "cfg" | "toml" | "pdb" | "ipynb" => Category::Code,
        "exe" | "dll" | "sys" | "msi" | "msix" | "appx" | "ocx" | "drv" | "scr" | "com" | "efi"
        | "so" | "dylib" | "jar" | "node" | "pyd" | "lib" => Category::Application,
        "pak" | "ucas" | "utoc" | "upk" | "uasset" | "umap" | "bundle" | "assets" | "ress" | "resource"
        | "vpk" | "bsa" | "ba2" | "forge" | "wad" | "rpf" | "pck" | "arc" | "big" | "sav" => Category::Game,
        "dat" | "db" | "sqlite" | "log" | "tmp" | "bin" | "etl" | "cache" | "idx" | "pf"
        | "evtx" | "regtrans-ms" | "blf" | "mui" | "cat" | "manifest" | "edb" | "jrs" | "chk" => {
            Category::System
        }
        _ => Category::Other,
    }
}
