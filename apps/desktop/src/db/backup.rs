//! Backups: stamped copies of the database, kept in a rolling set of ten.
//!
//! A backup is named after the last change in the file, so two backups of the
//! same state never collide and the newest one is the one to restore. The stamp
//! is read from `meta`, which is why this module asks sync.rs for it.

use super::*;

// ---------------------------------------------------------------------------
// Backup
// ---------------------------------------------------------------------------

/// How many backups are kept before the oldest is deleted.
pub(super) const BACKUP_KEEP: usize = 10;

/// The prefix every backup name carries. Also what tells a backup apart from
/// the live `pos.db` should the two ever share a directory.
pub(super) const BACKUP_PREFIX: &str = "pos-";

/// A filename-safe local timestamp, e.g. `20261101-184212`.
///
/// Asked of SQLite rather than the OS so a backup is named by the same clock
/// that stamps sales: two clocks an hour apart would make a file look newer or
/// older than the rows inside it, which is the one thing a restore cannot
/// afford.
pub fn stamp(store: &storage::Store) -> anyhow::Result<String> {
    store
        .conn()
        .query_row(
            "SELECT strftime('%Y%m%d-%H%M%S', 'now', 'localtime')",
            [],
            |row| row.get::<_, String>(0),
        )
        .context("reading the clock")
}

/// A consistent copy of the live database, pruned to the newest ten.
///
/// `VACUUM INTO` rather than a file copy: the till runs in WAL mode, and a plain
/// copy of the main file can miss committed pages still sitting in the log. The
/// snapshot it writes is a complete database on its own and is taken without
/// stopping the till.
///
/// Returns the file it wrote. Errors are returned rather than logged because the
/// operator pressed a button and is owed an answer.
pub fn backup(store: &storage::Store, dir: &Path) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("creating backup directory {}", dir.display()))?;

    let target = next_backup_path(store, dir)?;
    store
        .conn()
        .execute("VACUUM INTO ?1", [target.to_string_lossy().as_ref()])
        .with_context(|| format!("writing backup {}", target.display()))?;

    prune(dir);
    Ok(target)
}

/// A backup, unless the newest one is younger than `hours`.
///
/// The file name is the record: every backup is named by the local clock it was
/// taken with, so "when did we last copy this" needs no row of its own — and a
/// database restored from a backup answers it from the files beside it, which
/// is exactly when a fresh copy is wanted.
///
/// `None` means a recent copy is already there and nothing was written. Errors
/// are returned: a scheduled backup that did not happen has to reach a person.
pub fn backup_if_due(
    store: &storage::Store,
    dir: &Path,
    hours: i64,
) -> anyhow::Result<Option<PathBuf>> {
    let cutoff = stamp_before(store, hours)?;
    match newest_backup(dir)? {
        Some(newest) if newest >= cutoff => Ok(None),
        _ => backup(store, dir).map(Some),
    }
}

/// The local clock `hours` ago, in the sortable form [`stamp`] writes.
///
/// Asked of SQLite so the subtraction uses the same clock and the same daylight
/// rules as the formatting; two clocks disagreeing about an hour would make the
/// check wrong twice a year.
pub(super) fn stamp_before(store: &storage::Store, hours: i64) -> anyhow::Result<String> {
    let modifier = format!("-{hours} hours");
    store
        .conn()
        .query_row(
            "SELECT strftime('%Y%m%d-%H%M%S', 'now', 'localtime', ?1)",
            [modifier],
            |row| row.get::<_, String>(0),
        )
        .context("reading the clock")
}

/// The stamp of the newest backup in `dir`, if the directory has any.
///
/// Read from the name, not the modification time: the name is what a person
/// sorts by, and the sequence suffix means two backups in one second still
/// order the way they were taken.
pub(super) fn newest_backup(dir: &Path) -> anyhow::Result<Option<String>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        // A till that has never backed up has no folder yet. That is "none",
        // not a failure to look.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("listing {}", dir.display())),
    };

    let mut newest: Option<String> = None;
    for entry in entries.filter_map(Result::ok) {
        if !is_backup(&entry.path()) {
            continue;
        }
        let name = entry.file_name();
        let Some(rest) = name
            .to_str()
            .and_then(|name| name.strip_prefix(BACKUP_PREFIX))
        else {
            continue;
        };
        // `<YYYYMMDD>-<HHMMSS>-<NNNN>.db`: the stamp is the first two parts.
        let mut parts = rest.split('-');
        let (Some(day), Some(clock)) = (parts.next(), parts.next()) else {
            continue;
        };
        if day.len() != 8 || clock.len() != 6 {
            continue;
        }
        let stamp = format!("{day}-{clock}");
        if newest.as_ref().is_none_or(|current| stamp > *current) {
            newest = Some(stamp);
        }
    }
    Ok(newest)
}

/// The next name in this second's run.
///
/// The stamp has second resolution, so two backups in one second would otherwise
/// collide and `VACUUM INTO` refuses an existing file. A zero-padded sequence is
/// always present — not only on collision — because it is also what makes the
/// names sort: `…-0002` after `…-0001`, and every file of a later second after
/// every file of an earlier one. Pruning can then trust the name alone; a file
/// system's modification time is not precise enough to separate two backups
/// taken a moment apart.
///
/// The sequence continues past the highest number already used rather than
/// taking the first free one. Pruning leaves gaps at the start of the run, and
/// filling a gap would file the newest backup where the name sort expects the
/// oldest — exactly the file the next prune would delete.
pub(super) fn next_backup_path(store: &storage::Store, dir: &Path) -> anyhow::Result<PathBuf> {
    let stem = format!("{BACKUP_PREFIX}{}", stamp(store)?);
    let next = highest_sequence(dir, &stem)?.map_or(1, |highest| highest.saturating_add(1));
    if next > 9999 {
        anyhow::bail!("more than 9999 backups in one second; the clock or directory is wrong");
    }
    Ok(dir.join(format!("{stem}-{next:04}.db")))
}

/// The highest sequence already used by files named `<stem>-NNNN.db`.
pub(super) fn highest_sequence(dir: &Path, stem: &str) -> anyhow::Result<Option<u32>> {
    let entries = std::fs::read_dir(dir).with_context(|| format!("listing {}", dir.display()))?;
    Ok(entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let rest = name.to_str()?.strip_prefix(stem)?.strip_prefix('-')?;
            rest.strip_suffix(".db")?.parse().ok()
        })
        .max())
}

/// Delete everything past the newest [`BACKUP_KEEP`].
///
/// Ordered by name, which is chronological by construction (see
/// [`next_backup_path`]). Deletion failures are logged, not returned: the backup
/// succeeded, and reporting it as failed because an old file would not go away
/// would send the operator looking for a file that is there.
pub(super) fn prune(dir: &Path) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            tracing::warn!(error = %err, dir = %dir.display(), "could not list backups");
            return;
        }
    };

    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_backup(path))
        .collect();

    // Newest first: the last entry of a name sort is the most recent backup.
    files.sort();
    files.reverse();

    for path in files.into_iter().skip(BACKUP_KEEP) {
        if let Err(err) = std::fs::remove_file(&path) {
            tracing::warn!(error = %err, path = %path.display(), "could not delete an old backup");
        }
    }
}

pub(super) fn is_backup(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.strip_prefix(BACKUP_PREFIX).is_some() && path.extension().is_some_and(|ext| ext == "db")
}
