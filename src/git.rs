use anyhow::{anyhow, bail, Context, Result};
use chrono::Local;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Clone)]
pub struct GitRepo {
    pub worktree: PathBuf,
    pub gitdir: PathBuf,
    pub config: GitConfig,
}

#[derive(Default, Clone)]
pub struct GitConfig {
    core: HashMap<String, String>,
}

impl GitConfig {
    pub fn get_core(&self, key: &str) -> Option<&str> {
        self.core.get(key).map(|v| v.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectType {
    Blob,
    Tree,
    Commit,
    Tag,
}

impl ObjectType {
    fn as_str(self) -> &'static str {
        match self {
            ObjectType::Blob => "blob",
            ObjectType::Tree => "tree",
            ObjectType::Commit => "commit",
            ObjectType::Tag => "tag",
        }
    }

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "blob" => Ok(ObjectType::Blob),
            "tree" => Ok(ObjectType::Tree),
            "commit" => Ok(ObjectType::Commit),
            "tag" => Ok(ObjectType::Tag),
            _ => bail!("unknown object type: {s}"),
        }
    }
}

#[derive(Clone)]
pub struct IndexEntry {
    pub path: PathBuf,
    pub oid: String,
}

#[derive(Clone)]
pub struct Commit {
    pub tree: String,
    pub parent: Option<String>,
    pub author: String,
    pub committer: String,
    pub message: String,
}

pub fn repo_create(path: &Path) -> Result<GitRepo> {
    if path.exists() && !path.is_dir() {
        bail!("not a directory: {}", path.display());
    }
    fs::create_dir_all(path).context("create worktree")?;

    let gitdir = path.join(".git");
    if gitdir.exists() {
        bail!("git directory already exists: {}", gitdir.display());
    }
    fs::create_dir_all(gitdir.join("objects"))?;
    fs::create_dir_all(gitdir.join("refs").join("heads"))?;
    fs::create_dir_all(gitdir.join("refs").join("tags"))?;
    fs::write(gitdir.join("HEAD"), b"ref: refs/heads/main\n")?;
    fs::write(
        gitdir.join("config"),
        b"[core]\n\trepositoryformatversion = 0\n\tfilemode = false\n\tbare = false\n",
    )?;

    repo_find(path)
}

pub fn repo_find(start: &Path) -> Result<GitRepo> {
    let mut cur = start.canonicalize().context("canonicalize start path")?;
    loop {
        let gitdir = cur.join(".git");
        if gitdir.is_dir() {
            let config = read_config(&gitdir.join("config"))?;
            return Ok(GitRepo {
                worktree: cur,
                gitdir,
                config,
            });
        }
        if !cur.pop() {
            break;
        }
    }
    bail!("not inside a git repository")
}

fn read_config(path: &Path) -> Result<GitConfig> {
    if !path.exists() {
        return Ok(GitConfig::default());
    }
    let content = fs::read_to_string(path)?;
    let mut cfg = GitConfig::default();
    let mut section = String::new();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
            continue;
        }
        if let Some((key, val)) = line.split_once('=') {
            if section == "core" {
                cfg.core
                    .insert(key.trim().to_string(), val.trim().to_string());
            }
        }
    }
    Ok(cfg)
}

fn repo_path(repo: &GitRepo, parts: &[&str]) -> PathBuf {
    let mut path = repo.gitdir.clone();
    for part in parts {
        path.push(part);
    }
    path
}

fn repo_file(repo: &GitRepo, parts: &[&str], create: bool) -> Result<PathBuf> {
    let path = repo_path(repo, parts);
    if let Some(parent) = path.parent() {
        if create {
            fs::create_dir_all(parent)?;
        }
    }
    Ok(path)
}

pub fn object_read(repo: &GitRepo, oid: &str) -> Result<(ObjectType, Vec<u8>)> {
    let (dir, file) = oid.split_at(2);
    let path = repo_path(repo, &["objects", dir, file]);
    let file = fs::File::open(&path)
        .with_context(|| format!("object not found: {oid} ({})", path.display()))?;
    let mut decoder = ZlibDecoder::new(file);
    let mut raw = Vec::new();
    decoder.read_to_end(&mut raw)?;

    let null_pos = raw
        .iter()
        .position(|b| *b == 0)
        .ok_or_else(|| anyhow!("invalid object header"))?;
    let header = std::str::from_utf8(&raw[..null_pos])?;
    let (kind, size_str) = header
        .split_once(' ')
        .ok_or_else(|| anyhow!("invalid object header"))?;
    let size: usize = size_str.parse()?;
    let data = raw[null_pos + 1..].to_vec();
    if data.len() != size {
        bail!("malformed object: expected {size} bytes, got {}", data.len());
    }
    Ok((ObjectType::from_str(kind)?, data))
}

pub fn object_write(repo: &GitRepo, kind: ObjectType, data: &[u8], write: bool) -> Result<String> {
    let mut store = Vec::new();
    store.extend_from_slice(format!("{} {}\0", kind.as_str(), data.len()).as_bytes());
    store.extend_from_slice(data);

    let mut hasher = Sha1::new();
    hasher.update(&store);
    let oid = hex::encode(hasher.finalize());

    if write {
        let (dir, file) = oid.split_at(2);
        let path = repo_file(repo, &["objects", dir, file], true)?;
        if !path.exists() {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(&store)?;
            let compressed = encoder.finish()?;
            fs::write(path, compressed)?;
        }
    }

    Ok(oid)
}

pub fn cmd_init(path: &Path) -> Result<()> {
    repo_create(path)?;
    println!("Initialized empty vex repository in {}", path.display());
    Ok(())
}

pub fn cmd_hash_object(repo: &GitRepo, path: &Path, write: bool) -> Result<()> {
    let data = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let oid = object_write(repo, ObjectType::Blob, &data, write)?;
    println!("{oid}");
    Ok(())
}

pub fn cmd_cat_file(repo: &GitRepo, oid: &str) -> Result<()> {
    let (kind, data) = object_read(repo, oid)?;
    match kind {
        ObjectType::Blob => {
            print!("{}", String::from_utf8_lossy(&data));
        }
        ObjectType::Tree => {
            for entry in read_tree(&data)? {
                let entry_kind = if entry.mode == "40000" { "tree" } else { "blob" };
                println!(
                    "{} {} {}\t{}",
                    entry.mode, entry_kind, entry.oid, entry.path.display()
                );
            }
        }
        ObjectType::Commit | ObjectType::Tag => {
            print!("{}", String::from_utf8_lossy(&data));
        }
    }
    Ok(())
}

pub fn cmd_ls_tree(repo: &GitRepo, name: &str) -> Result<()> {
    let oid = resolve_treeish(repo, name)?;
    let (kind, data) = object_read(repo, &oid)?;
    let tree_data = match kind {
        ObjectType::Tree => data,
        ObjectType::Commit => {
            let commit = commit_read(&data)?;
            let (_, tree_data) = object_read(repo, &commit.tree)?;
            tree_data
        }
        _ => bail!("not a tree-ish object"),
    };

    for entry in read_tree(&tree_data)? {
        let entry_kind = if entry.mode == "40000" { "tree" } else { "blob" };
        println!(
            "{} {} {}\t{}",
            entry.mode, entry_kind, entry.oid, entry.path.display()
        );
    }
    Ok(())
}

pub fn cmd_add(repo: &GitRepo, paths: &[PathBuf]) -> Result<()> {
    let index = read_index(repo)?;
    let mut index_map: HashMap<String, IndexEntry> = index
        .into_iter()
        .map(|e| (e.path.to_string_lossy().to_string(), e))
        .collect();

    let mut to_add = Vec::new();
    for path in paths {
        let abs = repo.worktree.join(path);
        if abs.is_dir() {
            for entry in WalkDir::new(&abs)
                .into_iter()
                .filter_entry(|e| e.file_name() != ".git")
            {
                let entry = entry?;
                if entry.file_type().is_file() {
                    to_add.push(entry.path().to_path_buf());
                }
            }
        } else {
            to_add.push(abs);
        }
    }

    for file in to_add {
        if is_in_gitdir(repo, &file) {
            continue;
        }
        let rel = file
            .strip_prefix(&repo.worktree)
            .context("file is outside worktree")?;
        let data = fs::read(&file)?;
        let oid = object_write(repo, ObjectType::Blob, &data, true)?;
        index_map.insert(
            rel.to_string_lossy().to_string(),
            IndexEntry {
                path: rel.to_path_buf(),
                oid,
            },
        );
    }

    let mut entries: Vec<IndexEntry> = index_map.into_values().collect();
    entries.sort_by_key(|e| e.path.clone());
    write_index(repo, &entries)?;
    Ok(())
}

pub fn cmd_ls_files(repo: &GitRepo) -> Result<()> {
    let index = read_index(repo)?;
    for entry in index {
        println!("{}", entry.path.display());
    }
    Ok(())
}

pub fn cmd_status(repo: &GitRepo) -> Result<()> {
    let index = read_index(repo)?;
    let mut index_map = HashMap::new();
    for entry in &index {
        index_map.insert(entry.path.to_string_lossy().to_string(), entry.oid.clone());
    }

    let mut modified = Vec::new();
    let mut deleted = Vec::new();
    for entry in &index {
        let path = repo.worktree.join(&entry.path);
        if !path.exists() {
            deleted.push(entry.path.clone());
            continue;
        }
        let data = fs::read(&path)?;
        let oid = object_write(repo, ObjectType::Blob, &data, false)?;
        if oid != entry.oid {
            modified.push(entry.path.clone());
        }
    }

    let mut untracked = Vec::new();
    for entry in WalkDir::new(&repo.worktree)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
    {
        let entry = entry?;
        if entry.file_type().is_file() {
            let rel = entry.path().strip_prefix(&repo.worktree)?;
            let rel_str = rel.to_string_lossy().to_string();
            if !index_map.contains_key(&rel_str) {
                untracked.push(rel.to_path_buf());
            }
        }
    }

    if modified.is_empty() && deleted.is_empty() && untracked.is_empty() {
        println!("nothing to commit, working tree clean");
        return Ok(());
    }

    if !modified.is_empty() {
        println!("modified:");
        for path in modified {
            println!("  {}", path.display());
        }
    }
    if !deleted.is_empty() {
        println!("deleted:");
        for path in deleted {
            println!("  {}", path.display());
        }
    }
    if !untracked.is_empty() {
        println!("untracked:");
        for path in untracked {
            println!("  {}", path.display());
        }
    }
    Ok(())
}

pub fn cmd_rm(repo: &GitRepo, paths: &[PathBuf], cached: bool) -> Result<()> {
    let mut index = read_index(repo)?;
    let mut remove = Vec::new();
    for path in paths {
        remove.push(path.to_string_lossy().to_string());
    }
    index.retain(|entry| !remove.contains(&entry.path.to_string_lossy().to_string()));
    write_index(repo, &index)?;

    if !cached {
        for path in paths {
            let abs = repo.worktree.join(path);
            if abs.exists() {
                fs::remove_file(abs)?;
            }
        }
    }
    Ok(())
}

pub fn cmd_checkout(repo: &GitRepo, name: &str) -> Result<()> {
    let oid = resolve_treeish(repo, name)?;
    let (kind, data) = object_read(repo, &oid)?;
    let tree_oid = match kind {
        ObjectType::Tree => oid,
        ObjectType::Commit => commit_read(&data)?.tree,
        _ => bail!("not a tree-ish object"),
    };

    let existing = read_index(repo)?;
    for entry in existing {
        let path = repo.worktree.join(entry.path);
        if path.exists() {
            fs::remove_file(path)?;
        }
    }

    let mut entries = Vec::new();
    checkout_tree(repo, &tree_oid, &repo.worktree, &mut entries)?;
    write_index(repo, &entries)?;
    Ok(())
}

pub fn cmd_commit(repo: &GitRepo, message: &str) -> Result<()> {
    let index = read_index(repo)?;
    if index.is_empty() {
        bail!("nothing to commit (index is empty)");
    }
    let tree = write_tree(repo, &index)?;
    let parent = resolve_ref_optional(repo, "HEAD")?;
    let commit = commit_create(repo, &tree, parent.as_deref(), message)?;
    update_ref(repo, "HEAD", &commit)?;
    println!("{commit}");
    Ok(())
}

pub fn cmd_log(repo: &GitRepo, name: Option<&str>) -> Result<()> {
    let start = if let Some(name) = name {
        resolve_ref(repo, name)?
    } else {
        resolve_ref(repo, "HEAD")?
    };
    let mut cur = start;
    loop {
        let (kind, data) = object_read(repo, &cur)?;
        if kind != ObjectType::Commit {
            bail!("{cur} is not a commit");
        }
        let commit = commit_read(&data)?;
        println!("commit {cur}");
        println!("Author: {}", commit.author);
        println!("Date:   {}", commit.committer);
        println!();
        println!("    {}", commit.message.trim());
        println!();
        if let Some(parent) = commit.parent {
            cur = parent;
        } else {
            break;
        }
    }
    Ok(())
}

pub fn cmd_rev_parse(repo: &GitRepo, name: &str) -> Result<()> {
    let oid = resolve_ref(repo, name)?;
    println!("{oid}");
    Ok(())
}

pub fn cmd_show_ref(repo: &GitRepo, show_head: bool) -> Result<()> {
    if show_head {
        if let Ok(head) = resolve_ref(repo, "HEAD") {
            println!("{head} HEAD");
        }
    }
    for (name, oid) in list_refs(repo)? {
        println!("{oid} {name}");
    }
    Ok(())
}

pub fn cmd_tag(repo: &GitRepo, name: &str, target: Option<&str>) -> Result<()> {
    let oid = if let Some(target) = target {
        resolve_ref(repo, target)?
    } else {
        resolve_ref(repo, "HEAD")?
    };
    let path = repo_file(repo, &["refs", "tags", name], true)?;
    fs::write(path, format!("{oid}\n"))?;
    Ok(())
}

pub fn cmd_check_ignore(repo: &GitRepo, paths: &[PathBuf]) -> Result<()> {
    let ignore = load_gitignore(repo)?;
    for path in paths {
        let abs = repo.worktree.join(path);
        let rel = abs.strip_prefix(&repo.worktree).unwrap_or(&abs);
        let is_dir = abs.is_dir();
        if ignore
            .matched_path_or_any_parents(rel, is_dir)
            .is_ignore()
        {
            println!("{}", rel.display());
        }
    }
    Ok(())
}

fn load_gitignore(repo: &GitRepo) -> Result<Gitignore> {
    let mut builder = GitignoreBuilder::new(&repo.worktree);
    let gitignore = repo.worktree.join(".gitignore");
    if gitignore.exists() {
        builder.add(gitignore);
    }
    Ok(builder.build()?)
}

fn is_in_gitdir(repo: &GitRepo, path: &Path) -> bool {
    path.starts_with(&repo.gitdir)
}

fn read_index(repo: &GitRepo) -> Result<Vec<IndexEntry>> {
    let path = repo_file(repo, &["index"], false)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)?;
    let mut entries = Vec::new();
    for line in content.lines() {
        if let Some((oid, path)) = line.split_once('\t') {
            entries.push(IndexEntry {
                path: PathBuf::from(path),
                oid: oid.to_string(),
            });
        }
    }
    Ok(entries)
}

fn write_index(repo: &GitRepo, entries: &[IndexEntry]) -> Result<()> {
    let path = repo_file(repo, &["index"], true)?;
    let mut out = String::new();
    for entry in entries {
        out.push_str(&format!("{}\t{}\n", entry.oid, entry.path.display()));
    }
    fs::write(path, out)?;
    Ok(())
}

fn write_tree(repo: &GitRepo, entries: &[IndexEntry]) -> Result<String> {
    write_tree_inner(repo, entries, Path::new(""))
}

fn write_tree_inner(repo: &GitRepo, entries: &[IndexEntry], prefix: &Path) -> Result<String> {
    let mut files = Vec::new();
    let mut dirs: BTreeMap<String, Vec<IndexEntry>> = BTreeMap::new();

    for entry in entries {
        if let Ok(stripped) = entry.path.strip_prefix(prefix) {
            let mut comps = stripped.components();
            if let Some(first) = comps.next() {
                let name = first.as_os_str().to_string_lossy().to_string();
                if comps.next().is_none() {
                    files.push((name, entry.clone()));
                } else {
                    dirs.entry(name).or_default().push(entry.clone());
                }
            }
        }
    }

    let mut tree_entries = Vec::new();
    for (name, entry) in files {
        tree_entries.push(TreeEntry {
            mode: "100644".to_string(),
            path: PathBuf::from(name),
            oid: entry.oid,
        });
    }

    for (dir, sub_entries) in dirs {
        let dir_path = prefix.join(&dir);
        let sub_oid = write_tree_inner(repo, &sub_entries, &dir_path)?;
        tree_entries.push(TreeEntry {
            mode: "40000".to_string(),
            path: PathBuf::from(dir),
            oid: sub_oid,
        });
    }

    write_tree_object(repo, &tree_entries)
}

fn write_tree_object(repo: &GitRepo, entries: &[TreeEntry]) -> Result<String> {
    let mut data = Vec::new();
    for entry in entries {
        data.extend_from_slice(entry.mode.as_bytes());
        data.push(b' ');
        data.extend_from_slice(entry.path.to_string_lossy().as_bytes());
        data.push(0);
        let bytes = hex::decode(&entry.oid)?;
        data.extend_from_slice(&bytes);
    }
    object_write(repo, ObjectType::Tree, &data, true)
}

#[derive(Clone)]
struct TreeEntry {
    mode: String,
    path: PathBuf,
    oid: String,
}

fn read_tree(data: &[u8]) -> Result<Vec<TreeEntry>> {
    let mut entries = Vec::new();
    let mut idx = 0;
    while idx < data.len() {
        let mode_end = data[idx..]
            .iter()
            .position(|b| *b == b' ')
            .ok_or_else(|| anyhow!("invalid tree object"))?;
        let mode = std::str::from_utf8(&data[idx..idx + mode_end])?.to_string();
        idx += mode_end + 1;
        let name_end = data[idx..]
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| anyhow!("invalid tree object"))?;
        let name = std::str::from_utf8(&data[idx..idx + name_end])?.to_string();
        idx += name_end + 1;
        let oid = hex::encode(&data[idx..idx + 20]);
        idx += 20;
        entries.push(TreeEntry {
            mode,
            path: PathBuf::from(name),
            oid,
        });
    }
    Ok(entries)
}

fn commit_create(
    repo: &GitRepo,
    tree: &str,
    parent: Option<&str>,
    message: &str,
) -> Result<String> {
    let author = signature("GIT_AUTHOR_NAME", "GIT_AUTHOR_EMAIL")?;
    let committer = signature("GIT_COMMITTER_NAME", "GIT_COMMITTER_EMAIL")?;
    let mut content = String::new();
    content.push_str(&format!("tree {tree}\n"));
    if let Some(parent) = parent {
        content.push_str(&format!("parent {parent}\n"));
    }
    content.push_str(&format!("author {author}\n"));
    content.push_str(&format!("committer {committer}\n"));
    content.push('\n');
    content.push_str(message);
    if !message.ends_with('\n') {
        content.push('\n');
    }
    object_write(repo, ObjectType::Commit, content.as_bytes(), true)
}

fn signature(name_key: &str, email_key: &str) -> Result<String> {
    let name = env::var(name_key)
        .or_else(|_| env::var("USER"))
        .unwrap_or_else(|_| "unknown".to_string());
    let email = env::var(email_key).unwrap_or_else(|_| "unknown@example.com".to_string());
    let now = Local::now();
    Ok(format!(
        "{} <{}> {} {}",
        name,
        email,
        now.timestamp(),
        now.format("%z")
    ))
}

fn commit_read(data: &[u8]) -> Result<Commit> {
    let content = String::from_utf8_lossy(data);
    let (headers, message) = content
        .split_once("\n\n")
        .ok_or_else(|| anyhow!("invalid commit object"))?;
    let mut tree = None;
    let mut parent = None;
    let mut author = None;
    let mut committer = None;
    for line in headers.lines() {
        if let Some(value) = line.strip_prefix("tree ") {
            tree = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("parent ") {
            parent = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("author ") {
            author = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("committer ") {
            committer = Some(value.to_string());
        }
    }
    Ok(Commit {
        tree: tree.ok_or_else(|| anyhow!("commit missing tree"))?,
        parent,
        author: author.unwrap_or_else(|| "unknown".to_string()),
        committer: committer.unwrap_or_else(|| "unknown".to_string()),
        message: message.to_string(),
    })
}

fn resolve_ref(repo: &GitRepo, name: &str) -> Result<String> {
    if name == "HEAD" {
        let head = fs::read_to_string(repo_path(repo, &["HEAD"]))?;
        let head = head.trim();
        if let Some(ref_name) = head.strip_prefix("ref: ") {
            return resolve_ref(repo, ref_name);
        }
        return Ok(head.to_string());
    }

    if name.len() == 40 && name.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(name.to_string());
    }

    let ref_path = repo_path(repo, &[name]);
    if ref_path.exists() {
        let oid = fs::read_to_string(ref_path)?;
        return Ok(oid.trim().to_string());
    }

    let heads = repo_path(repo, &["refs", "heads", name]);
    if heads.exists() {
        let oid = fs::read_to_string(heads)?;
        return Ok(oid.trim().to_string());
    }

    let tags = repo_path(repo, &["refs", "tags", name]);
    if tags.exists() {
        let oid = fs::read_to_string(tags)?;
        return Ok(oid.trim().to_string());
    }

    bail!("unknown revision: {name}")
}

fn resolve_ref_optional(repo: &GitRepo, name: &str) -> Result<Option<String>> {
    match resolve_ref(repo, name) {
        Ok(oid) => Ok(Some(oid)),
        Err(err) if err.to_string().contains("unknown revision") => Ok(None),
        Err(err) => Err(err),
    }
}

fn update_ref(repo: &GitRepo, name: &str, oid: &str) -> Result<()> {
    if name == "HEAD" {
        let head = fs::read_to_string(repo_path(repo, &["HEAD"]))?;
        if let Some(ref_name) = head.trim().strip_prefix("ref: ") {
            let path = repo_file(repo, &[ref_name], true)?;
            fs::write(path, format!("{oid}\n"))?;
            return Ok(());
        }
    }
    let path = repo_file(repo, &[name], true)?;
    fs::write(path, format!("{oid}\n"))?;
    Ok(())
}

fn resolve_treeish(repo: &GitRepo, name: &str) -> Result<String> {
    let oid = resolve_ref(repo, name)?;
    let (kind, data) = object_read(repo, &oid)?;
    match kind {
        ObjectType::Tree => Ok(oid),
        ObjectType::Commit => Ok(commit_read(&data)?.tree),
        _ => bail!("not a tree-ish object"),
    }
}

fn list_refs(repo: &GitRepo) -> Result<Vec<(String, String)>> {
    let mut refs = Vec::new();
    let ref_root = repo_path(repo, &["refs"]);
    if !ref_root.exists() {
        return Ok(refs);
    }
    for entry in WalkDir::new(&ref_root) {
        let entry = entry?;
        if entry.file_type().is_file() {
            let oid = fs::read_to_string(entry.path())?;
            let rel = entry.path().strip_prefix(&repo.gitdir)?;
            refs.push((rel.to_string_lossy().to_string(), oid.trim().to_string()));
        }
    }
    refs.sort_by_key(|(name, _)| name.clone());
    Ok(refs)
}

fn checkout_tree(
    repo: &GitRepo,
    oid: &str,
    base: &Path,
    entries: &mut Vec<IndexEntry>,
) -> Result<()> {
    let (kind, data) = object_read(repo, oid)?;
    if kind != ObjectType::Tree {
        bail!("expected tree object");
    }
    for entry in read_tree(&data)? {
        let path = base.join(&entry.path);
        if entry.mode == "40000" {
            fs::create_dir_all(&path)?;
            checkout_tree(repo, &entry.oid, &path, entries)?;
        } else {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let (kind, blob_data) = object_read(repo, &entry.oid)?;
            if kind != ObjectType::Blob {
                bail!("tree entry is not a blob");
            }
            fs::write(&path, blob_data)?;
            let rel = path.strip_prefix(&repo.worktree)?;
            entries.push(IndexEntry {
                path: rel.to_path_buf(),
                oid: entry.oid.clone(),
            });
        }
    }
    Ok(())
}
