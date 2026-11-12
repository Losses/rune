//! Replay recorded successful SQLite writes and durability barriers.
//! This model assumes truthful file/directory syncs and atomic namespace operations;
//! it explores bounded losses, reordering and torn pending writes, not arbitrary damage.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashSet},
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    rc::Rc,
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;

const TEST_DEADLINE: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
struct Event {
    op: u8,
    kind: u8,
    offset: u64,
    payload: Vec<u8>,
}
fn read_trace(path: &Path) -> Result<Vec<Event>> {
    let bytes = fs::read(path)?;
    let mut pos = 0;
    let mut events = Vec::new();
    while pos < bytes.len() {
        ensure!(bytes.len() - pos >= 18, "truncated trace header");
        let op = bytes[pos];
        let kind = bytes[pos + 1];
        let offset = u64::from_le_bytes(bytes[pos + 2..pos + 10].try_into()?);
        let len = usize::try_from(u64::from_le_bytes(bytes[pos + 10..pos + 18].try_into()?))?;
        pos += 18;
        let payload = if op == b'W' {
            ensure!(len <= bytes.len() - pos, "truncated trace payload");
            let result = bytes[pos..pos + len].to_vec();
            pos += len;
            result
        } else {
            ensure!(len == 0, "unexpected trace payload length");
            Vec::new()
        };
        ensure!(
            matches!(op, b'C' | b'U' | b'S' | b'T' | b'W') && matches!(kind, b'D' | b'J' | b'P'),
            "unknown trace event"
        );
        events.push(Event {
            op,
            kind,
            offset,
            payload,
        });
    }
    Ok(events)
}
fn change(data: &mut Vec<u8>, op: u8, offset: u64, payload: &[u8]) -> Result<()> {
    let offset = usize::try_from(offset)?;
    if op == b'T' {
        data.resize(offset, 0);
        return Ok(());
    }
    ensure!(op == b'W', "unexpected mutation");
    let end = offset
        .checked_add(payload.len())
        .context("write offset overflow")?;
    if data.len() < end {
        data.resize(end, 0);
    }
    data[offset..end].copy_from_slice(payload);
    Ok(())
}
#[derive(Clone)]
struct Inode {
    cached: Vec<u8>,
    durable: Vec<u8>,
    pending: Vec<Event>,
}
struct Storage {
    live: BTreeMap<u8, Rc<RefCell<Inode>>>,
    names: BTreeMap<u8, Rc<RefCell<Inode>>>,
}
impl Storage {
    fn new(initial: Vec<u8>) -> Self {
        let inode = Rc::new(RefCell::new(Inode {
            cached: initial.clone(),
            durable: initial,
            pending: vec![],
        }));
        let live = BTreeMap::from([(b'D', inode)]);
        Self {
            names: live.clone(),
            live,
        }
    }
    fn apply(&mut self, e: &Event) -> Result<()> {
        match (e.op, e.kind) {
            (b'C', kind) => {
                self.live.insert(
                    kind,
                    Rc::new(RefCell::new(Inode {
                        cached: vec![],
                        durable: vec![],
                        pending: vec![],
                    })),
                );
            }
            (b'U', kind) => {
                self.live.remove(&kind);
            }
            (b'S', b'P') => self.names = self.live.clone(),
            (b'S', kind) => {
                let mut node = self
                    .live
                    .get(&kind)
                    .context("sync of missing inode")?
                    .borrow_mut();
                node.durable = node.cached.clone();
                node.pending.clear();
            }
            (_, kind) => {
                let mut node = self
                    .live
                    .get(&kind)
                    .context("mutation of missing inode")?
                    .borrow_mut();
                change(&mut node.cached, e.op, e.offset, &e.payload)?;
                node.pending.push(e.clone());
            }
        }
        Ok(())
    }
    fn image(
        &self,
        durable_namespace: bool,
        modes: &BTreeMap<u8, u8>,
    ) -> Result<BTreeMap<u8, Vec<u8>>> {
        let mut result = BTreeMap::new();
        for (kind, inode) in if durable_namespace {
            &self.names
        } else {
            &self.live
        } {
            let node = inode.borrow();
            let mut data = node.durable.clone();
            match modes
                .get(kind)
                .copied()
                .context("missing persistence mode")?
            {
                0 => {}
                1 => data = node.cached.clone(),
                2 => {
                    for event in node.pending.iter().step_by(2).rev() {
                        change(&mut data, event.op, event.offset, &event.payload)?;
                    }
                }
                3 => {
                    if let Some(event) = node.pending.last()
                        && event.op == b'W'
                    {
                        change(
                            &mut data,
                            event.op,
                            event.offset,
                            &event.payload[..(event.payload.len() / 2).max(1)],
                        )?;
                    }
                }
                _ => bail!("unknown persistence mode"),
            }
            result.insert(*kind, data);
        }
        Ok(result)
    }
}
fn model_checks() -> Result<()> {
    let mut independent = Storage::new(b"old".to_vec());
    independent.apply(&Event {
        op: b'C',
        kind: b'J',
        offset: 0,
        payload: vec![],
    })?;
    independent.apply(&Event {
        op: b'W',
        kind: b'J',
        offset: 0,
        payload: b"undo".to_vec(),
    })?;
    independent.apply(&Event {
        op: b'W',
        kind: b'D',
        offset: 0,
        payload: b"new".to_vec(),
    })?;
    assert_eq!(
        independent.image(false, &BTreeMap::from([(b'D', 1), (b'J', 0)]))?,
        BTreeMap::from([(b'D', b"new".to_vec()), (b'J', vec![])])
    );
    assert_eq!(
        independent.image(false, &BTreeMap::from([(b'D', 0), (b'J', 1)]))?,
        BTreeMap::from([(b'D', b"old".to_vec()), (b'J', b"undo".to_vec())])
    );
    let mut disk = Storage::new(b"old-data".to_vec());
    disk.apply(&Event {
        op: b'W',
        kind: b'D',
        offset: 0,
        payload: b"NEW".to_vec(),
    })?;
    assert_eq!(
        disk.image(true, &BTreeMap::from([(b'D', 0)]))?[&b'D'],
        b"old-data"
    );
    assert_eq!(
        disk.image(true, &BTreeMap::from([(b'D', 1)]))?[&b'D'],
        b"NEW-data"
    );
    disk.apply(&Event {
        op: b'S',
        kind: b'D',
        offset: 0,
        payload: vec![],
    })?;
    assert_eq!(
        disk.image(true, &BTreeMap::from([(b'D', 0)]))?[&b'D'],
        b"NEW-data"
    );
    disk.apply(&Event {
        op: b'C',
        kind: b'J',
        offset: 0,
        payload: vec![],
    })?;
    disk.apply(&Event {
        op: b'W',
        kind: b'J',
        offset: 0,
        payload: b"undo".to_vec(),
    })?;
    assert_eq!(
        disk.image(false, &BTreeMap::from([(b'D', 1), (b'J', 0)]))?[&b'J'],
        b""
    );
    assert_eq!(
        disk.image(false, &BTreeMap::from([(b'D', 0), (b'J', 1)]))?[&b'J'],
        b"undo"
    );
    disk.apply(&Event {
        op: b'S',
        kind: b'J',
        offset: 0,
        payload: vec![],
    })?;
    assert!(
        !disk
            .image(true, &BTreeMap::from([(b'D', 0)]))?
            .contains_key(&b'J')
    );
    disk.apply(&Event {
        op: b'S',
        kind: b'P',
        offset: 0,
        payload: vec![],
    })?;
    disk.apply(&Event {
        op: b'U',
        kind: b'J',
        offset: 0,
        payload: vec![],
    })?;
    assert_eq!(
        disk.image(true, &BTreeMap::from([(b'D', 0), (b'J', 0)]))?[&b'J'],
        b"undo"
    );
    disk.apply(&Event {
        op: b'S',
        kind: b'P',
        offset: 0,
        payload: vec![],
    })?;
    assert!(
        !disk
            .image(true, &BTreeMap::from([(b'D', 0)]))?
            .contains_key(&b'J')
    );
    Ok(())
}
fn wait(mut child: std::process::Child, limit: Duration) -> Result<std::process::Output> {
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return Ok(child.wait_with_output()?);
        }
        if start.elapsed() > limit {
            child.kill()?;
            child.wait()?;
            bail!("subprocess timed out");
        }
        thread::yield_now();
    }
}
fn run_case(binary: &Path, name: &str, root: &Path, extra: &[(&str, OsString)]) -> Result<String> {
    let mut command = Command::new(binary);
    command
        .args(["--ignored", "--exact", name, "--nocapture"])
        .env("RUNE_SYNC_CRASH_ROOT", root)
        .env("RUST_BACKTRACE", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in extra {
        command.env(key, value);
    }
    let output = wait(command.spawn()?, TEST_DEADLINE)?;
    ensure!(
        output.status.success(),
        "{name} failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}
fn binary() -> Result<PathBuf> {
    let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args([
            "test",
            "-p",
            "database",
            "--features",
            "sync-crash-probe",
            "--test",
            "sync_crash_recovery",
            "--offline",
            "--no-run",
            "--message-format=json",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()?;
    ensure!(
        output.status.success(),
        "building crash fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    for line in String::from_utf8(output.stdout)?.lines() {
        let v: Value = serde_json::from_str(line)?;
        if v["target"]["name"] == "sync_crash_recovery"
            && let Some(path) = v["executable"].as_str()
        {
            return Ok(path.into());
        }
    }
    bail!("missing crash fixture executable")
}
fn capture(binary: &Path, root: &Path) -> Result<(PathBuf, Value, Vec<Event>, String)> {
    let seed = root.join("seed");
    fs::create_dir(&seed)?;
    let setup = run_case(binary, "crash_probe_setup", &seed, &[])?;
    let manifest: Value = serde_json::from_slice(&fs::read(seed.join("manifest.json"))?)?;
    let baseline = root.join("baseline");
    copy_tree(&seed, &baseline)?;
    let shim = root.join("storage_trace.so");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/storage_trace.rs");
    let output = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .args([
            "--crate-type",
            "cdylib",
            "--edition",
            "2021",
            "-C",
            "opt-level=2",
            "-C",
            "panic=abort",
        ])
        .arg(source)
        .arg("-o")
        .arg(&shim)
        .output()?;
    ensure!(
        output.status.success(),
        "building recorder failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let trace = root.join("io.bin");
    let marker = root.join("committed");
    let value = |key: &str| -> Result<&str> {
        manifest[key]
            .as_str()
            .with_context(|| format!("missing {key}"))
    };
    let mut child = Command::new(binary)
        .args(["--ignored", "--exact", "crash_probe_child", "--nocapture"])
        .env("LD_PRELOAD", &shim)
        .env("RUNE_SYNC_STORAGE_DB", value("database")?)
        .env("RUNE_SYNC_STORAGE_TRACE", &trace)
        .env("RUNE_SYNC_CRASH_ROOT", &seed)
        .env("RUNE_SYNC_CRASH_URL", value("url")?)
        .env("RUNE_SYNC_CRASH_LIBRARY", value("library")?)
        .env("RUNE_SYNC_CRASH_NODE", value("node")?)
        .env("RUNE_SYNC_FAULT_STAGE", "page_after_commit")
        .env("RUNE_SYNC_FAULT_MARKER", &marker)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let started = Instant::now();
    while !marker.exists() {
        if let Some(status) = child.try_wait()? {
            let output = child.wait_with_output()?;
            bail!(
                "capture exited {status}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        if started.elapsed() > TEST_DEADLINE {
            child.kill()?;
            child.wait()?;
            bail!("capture timed out");
        }
        thread::yield_now();
    }
    child.kill()?;
    child.wait()?;
    let events = read_trace(&trace)?;
    for (op, kind) in [
        (b'W', b'D'),
        (b'W', b'J'),
        (b'S', b'D'),
        (b'S', b'J'),
        (b'S', b'P'),
        (b'U', b'J'),
    ] {
        ensure!(
            events.iter().any(|e| e.op == op && e.kind == kind),
            "required trace event missing: {}{}",
            op as char,
            kind as char
        );
    }
    for op in *b"CU" {
        ensure!(
            events
                .iter()
                .filter(|e| e.op == op && e.kind == b'J')
                .count()
                == 1,
            "expected exactly one journal generation"
        );
    }
    Ok((baseline, manifest, events, setup))
}
fn verify(
    binary: &Path,
    baseline: &Path,
    manifest: &Value,
    image: &BTreeMap<u8, Vec<u8>>,
    work: &Path,
    expected: &str,
) -> Result<()> {
    let recover = TempDir::new_in(work)?;
    copy_tree(baseline, recover.path())?;
    let database = recover.path().join(".rune/.0.db");
    let journal = PathBuf::from(format!("{}-journal", database.display()));
    for (kind, path) in [(b'D', &database), (b'J', &journal)] {
        if let Some(bytes) = image.get(&kind) {
            fs::write(path, bytes)?;
        } else if path.exists() {
            fs::remove_file(path)?;
        }
    }
    let mut m = manifest.clone();
    m["database"] = json!(database);
    m["url"] = json!(format!("sqlite://{}?mode=rwc", database.display()));
    fs::write(
        recover.path().join("manifest.json"),
        serde_json::to_vec(&m)?,
    )?;
    run_case(
        binary,
        "crash_probe_verify",
        recover.path(),
        &[("RUNE_SYNC_EXPECT_COMMITTED", expected.into())],
    )?;
    Ok(())
}
pub fn run() -> Result<()> {
    model_checks()?;
    let started = Instant::now();
    let temp = TempDir::new()?;
    let stable = temp.path().join("crash_probe");
    fs::copy(binary()?, &stable)?;
    let (baseline, manifest, events, setup) = capture(&stable, temp.path())?;
    let initial = fs::read(baseline.join(".rune/.0.db"))?;
    let mut disk = Storage::new(initial);
    let mut seen = HashSet::new();
    let mut images = 0usize;
    for (index, event) in events.iter().enumerate() {
        disk.apply(event)?;
        for durable in [true, false] {
            let kinds: Vec<u8> = if durable {
                disk.names.keys().copied().collect()
            } else {
                disk.live.keys().copied().collect()
            };
            for choice in 0..4usize.pow(kinds.len() as u32) {
                let mut n = choice;
                let mut modes = BTreeMap::new();
                for kind in &kinds {
                    modes.insert(*kind, (n % 4) as u8);
                    n /= 4;
                }
                let image = disk.image(durable, &modes)?;
                let expected = if index + 1 == events.len() {
                    "true"
                } else {
                    "auto"
                };
                let mut hash = blake3::Hasher::new();
                hash.update(expected.as_bytes());
                for (kind, data) in &image {
                    hash.update(&[*kind]);
                    hash.update(&(data.len() as u64).to_le_bytes());
                    hash.update(data);
                }
                if !seen.insert(hash.finalize()) {
                    continue;
                }
                verify(&stable,&baseline,&manifest,&image,temp.path(),expected).with_context(||format!("recovery failed at cut {} of {}, event {event:?}, durable_namespace={durable}, modes={modes:?}",index+1,events.len()))?;
                images += 1;
            }
        }
        if (index + 1) % 25 == 0 {
            println!(
                "validated cut {}/{}, images={images}",
                index + 1,
                events.len()
            );
        }
    }
    let file_syncs = events
        .iter()
        .filter(|e| e.op == b'S' && (e.kind == b'D' || e.kind == b'J'))
        .count();
    let directory_syncs = events
        .iter()
        .filter(|e| e.op == b'S' && e.kind == b'P')
        .count();
    println!(
        "{} operations={} file_syncs={file_syncs} directory_syncs={directory_syncs} cut_points={} recovered_images={images} elapsed_ms={}",
        setup
            .lines()
            .find(|line| line.starts_with("sqlite_"))
            .unwrap_or("sqlite settings unavailable"),
        events.len(),
        events.len(),
        started.elapsed().as_millis()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_incomplete_or_unrecognized_trace() -> Result<()> {
        let temp = TempDir::new()?;
        let file = temp.path().join("events");
        fs::write(&file, *b"WD")?;
        assert!(read_trace(&file).is_err());
        let mut truncated = vec![b'W', b'D'];
        truncated.extend_from_slice(&0u64.to_le_bytes());
        truncated.extend_from_slice(&4u64.to_le_bytes());
        truncated.extend_from_slice(b"ab");
        fs::write(&file, truncated)?;
        assert!(read_trace(&file).is_err());
        let mut unknown = vec![b'X', b'D'];
        unknown.extend_from_slice(&0u64.to_le_bytes());
        unknown.extend_from_slice(&0u64.to_le_bytes());
        fs::write(&file, unknown)?;
        assert!(read_trace(&file).is_err());
        Ok(())
    }
}
