//! Staff rows: who can sign in, whether they are active, and the attempt log
//! that rate-limits a guessed PIN.

use super::*;

/// Every account, owner first, then managers, then cashiers.
///
/// Switched-off accounts are included: Settings has to list them to switch them
/// back on, and the lock screen filters them out itself.
pub fn users(store: &storage::Store) -> Vec<User> {
    let mut statement = match store.conn().prepare(
        "SELECT id, name, role, credential, kind, active
         FROM users
         ORDER BY CASE role WHEN 'owner' THEN 0 WHEN 'manager' THEN 1 ELSE 2 END,
                  name COLLATE NOCASE",
    ) {
        Ok(statement) => statement,
        Err(err) => {
            tracing::warn!(error = %err, "users prepare failed");
            return Vec::new();
        }
    };

    let rows = statement.query_map([], |row| {
        let role: String = row.get(2)?;
        let kind: String = row.get(4)?;
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            role,
            row.get::<_, String>(3)?,
            kind,
            row.get::<_, bool>(5)?,
        ))
    });

    let Ok(rows) = rows else {
        tracing::warn!("users query failed");
        return Vec::new();
    };

    rows.filter_map(Result::ok)
        .filter_map(|(id, name, role, credential, kind, active)| {
            Some(User {
                id,
                name,
                role: crate::auth::Role::from_key(&role)?,
                credential,
                kind: crate::auth::CredentialKind::from_key(&kind)?,
                active,
            })
        })
        .collect()
}

/// The owner account, which exists once the till has been set up.
pub fn owner(store: &storage::Store) -> Option<User> {
    users(store)
        .into_iter()
        .find(|user| user.role == crate::auth::Role::Owner)
}

/// Add an account and return it, id included.
pub fn insert_user(
    store: &storage::Store,
    name: &str,
    role: crate::auth::Role,
    kind: crate::auth::CredentialKind,
    credential: &str,
) -> anyhow::Result<User> {
    let id = uuid::Uuid::now_v7().to_string();
    store.conn().execute(
        "INSERT INTO users (id, name, role, credential, kind) VALUES (?1, ?2, ?3, ?4, ?5)",
        (id.as_str(), name, role.key(), credential, kind.key()),
    )?;

    Ok(User {
        id,
        name: name.to_owned(),
        role,
        credential: credential.to_owned(),
        kind,
        active: true,
    })
}

/// Switch an account on or off. Accounts are never deleted: the sales they took
/// still name them.
pub fn set_user_active(store: &storage::Store, id: &str, active: bool) -> anyhow::Result<()> {
    let changed = store
        .conn()
        .execute("UPDATE users SET active = ?2 WHERE id = ?1", (id, active))?;
    anyhow::ensure!(changed == 1, "that account no longer exists");
    Ok(())
}

/// Replace an account's secret. The kind does not change with it.
pub fn set_user_credential(
    store: &storage::Store,
    id: &str,
    credential: &str,
) -> anyhow::Result<()> {
    let changed = store.conn().execute(
        "UPDATE users SET credential = ?2 WHERE id = ?1",
        (id, credential),
    )?;
    anyhow::ensure!(changed == 1, "that account no longer exists");
    Ok(())
}

/// Failures on record for an account, and the second it is locked until.
pub fn attempts(store: &storage::Store, user_id: &str) -> (u32, Option<i64>) {
    store
        .conn()
        .query_row(
            "SELECT failures, locked_until FROM login_attempts WHERE user_id = ?1",
            [user_id],
            |row| Ok((row.get::<_, i64>(0)? as u32, row.get(1)?)),
        )
        .unwrap_or((0, None))
}

/// Remember a failed attempt and the lock it earned.
pub fn record_attempt(
    store: &storage::Store,
    user_id: &str,
    failures: u32,
    locked_until: Option<i64>,
) {
    if let Err(err) = store.conn().execute(
        "INSERT INTO login_attempts (user_id, failures, locked_until) VALUES (?1, ?2, ?3)
         ON CONFLICT(user_id) DO UPDATE SET failures = excluded.failures,
                                            locked_until = excluded.locked_until",
        (user_id, failures as i64, locked_until),
    ) {
        // A till that cannot remember failures still lets the right PIN in; it
        // only loses the throttle, which is better than refusing everyone.
        tracing::error!(error = %err, user = %user_id, "could not record a failed sign-in");
    }
}

/// A correct secret clears the record, so yesterday's mistakes do not count.
pub fn clear_attempts(store: &storage::Store, user_id: &str) {
    if let Err(err) = store
        .conn()
        .execute("DELETE FROM login_attempts WHERE user_id = ?1", [user_id])
    {
        tracing::error!(error = %err, user = %user_id, "could not clear failed sign-ins");
    }
}
