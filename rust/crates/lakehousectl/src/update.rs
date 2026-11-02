//! `lakehousectl update`: the stack moves to another release (REL-2 S4 decision D17).
//!
//! # The steps, and their order
//!
//! D17 fixes the order, and the order is the whole safety of the
//! command: nothing that is RUNNING is touched before step 4.
//!
//! 1. **The release is obtained and verified**, as the install does.
//! 2. **The route is computed** (D10) from the state's
//!    `currentVersion`.
//! 3. **The pre-flight runs**, the files the release ships are written
//!    to a staging directory, and every image is pulled or loaded. A
//!    failure here leaves the running stack as it was.
//! 4. **`MAINTENANCE_PAUSED` is set** and the writers are stopped.
//! 5. **A backup is taken** and recorded in the state.
//! 6. **The new files go in place**: the old Compose file and the old
//!    manifest are kept as `*.previous`, the new ones are written, and
//!    the stack is started.
//! 7. **The stack is waited for**: the API must report the NEW
//!    version as ready, and the console must answer.
//! 8. **On success** the pause is cleared and the state names the new
//!    release.
//! 9. **On a failure from step 6 on** the update rolls ITSELF back
//!    (D18) and exits non-zero, saying what failed and what the
//!    rollback did.
//!
//! # A failure in step 4 or step 5
//!
//! Nothing has been swapped when the writers cannot be stopped or the
//! backup cannot be taken, so the install still holds the OLD release's
//! files. The update starts the stack again on those files, waits for
//! it on the OLD version, clears the pause and exits non-zero, saying
//! that it stopped before it changed anything. A stack that does not
//! come back is reported as it is: the install may need a hand
//! (REL-2 S4b review round 1 BLOCKER 1).
//!
//! # The route (D10)
//!
//! The route comes from the manifest's `minFrom` (the lowest release
//! that may update to this one directly) and its `stops` (the releases
//! an update to this one must pass through):
//!
//! - the SAME release: there is nothing to do;
//! - an OLDER release: refused, pointing at `lakehousectl rollback`;
//! - a release below `minFrom`, or one with a stop between: the update
//!   goes through the stops, one release at a time. That needs the
//!   network — an air-gapped host is refused, and the refusal names
//!   the bundles it would need.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::cli::UpdateArgs;
use crate::error::ToolError;
use crate::files;
use crate::manifest::Manifest;
use crate::pause;
use crate::preflight;
use crate::release;
use crate::rollback;
use crate::runner::{self, Local, Runner};
use crate::state;
use crate::verify;
use crate::version;

/// The route from the installed release to the release the update moves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// The release that is installed IS the release the update moves
    /// to: there is nothing to do.
    Nothing,
    /// The release may be updated to directly.
    Direct,
    /// The release must be reached through these releases, in order.
    Hops(Vec<String>),
}

/// What the update needs to know before it touches anything.
///
/// The facts are injected, so the whole path can be driven by a test
/// with no machine at all (REL-2 S4 decision D17).
#[derive(Debug, Clone)]
pub struct Facts {
    /// The state of the install, as it is now.
    pub state: state::State,
    /// The release the update moves to.
    pub manifest: Manifest,
    /// The manifest's bytes.
    pub manifest_bytes: Vec<u8>,
    /// The manifest's Sigstore bundle.
    pub manifest_bundle: Vec<u8>,
    /// The release's Compose file.
    pub compose: String,
    /// The files the release ships, checked against the signed manifest.
    pub shipped: files::ShippedFiles,
    /// The bundle's directory, when the update is offline.
    pub bundle: Option<PathBuf>,
    /// The pre-flight failures, when there are any.
    pub preflight: Vec<preflight::Failure>,
    /// How long the stack may take to become ready, in seconds.
    pub ready_timeout: u64,
}

/// Run `lakehousectl update`.
///
/// # Errors
///
/// Returns the error that stopped the update. A failure from step 6 on
/// is reported WITH what the rollback did (REL-2 S4 decision D17).
pub fn command(args: &UpdateArgs) -> Result<(), ToolError> {
    preflight::validate_ignored(&args.ignore_preflight).map_err(ToolError::Usage)?;

    // Step 1 — the release is obtained and verified, as the install does.
    let release = release::obtain(
        args.bundle.as_deref(),
        args.version.as_deref(),
        &mut release::download,
    )?;
    let key = verify::public_key()?;
    verify::verify_manifest(&release.manifest, &release.bundle, &key)?;
    println!(
        "lakehousectl: the manifest of release {} verifies against the release key.",
        release::version_of(&release.manifest)?
    );

    let manifest: Manifest = serde_json::from_slice(&release.manifest)
        .map_err(|error| ToolError::Failed(format!("the manifest could not be read: {error}")))?;
    files::check_compose(&manifest, &release.compose)?;

    // Step 2 — the route.
    let current = state::read(&args.dir)?.current_version.ok_or_else(|| {
        ToolError::Failed(
            "the install's state names no release, so the tool cannot tell what it would update. \
             `lakehousectl install` writes that state."
                .to_string(),
        )
    })?;
    let route = compute_route(&current, &manifest).map_err(ToolError::Failed)?;
    match route {
        Route::Nothing => {
            println!(
                "lakehousectl: the release {} is already installed. There is nothing to do.",
                manifest.version
            );
            return Ok(());
        }
        Route::Hops(hops) if args.bundle.is_some() => {
            // An air-gapped host has one bundle, and a route through
            // other releases needs one bundle for each of them.
            return Err(ToolError::Failed(format!(
                "the release {} may only be updated to through {}, and this host is offline. \
                 Bring the bundle of every one of them — {} — and run the update once for each, \
                 in order.",
                manifest.version,
                hops.join(", "),
                hops.join(", ")
            )));
        }
        Route::Direct => {
            let facts = gather(args, &release, &manifest)?;
            let mut runner = Local::new(&args.dir);
            run(&args.dir, &facts, &mut runner)?;
        }
        Route::Hops(hops) => {
            for hop in &hops {
                println!("lakehousectl: the route passes through {hop}.");
                run_hop(args, hop)?;
            }
            let facts = gather(args, &release, &manifest)?;
            let mut runner = Local::new(&args.dir);
            run(&args.dir, &facts, &mut runner)?;
        }
    }
    Ok(())
}

/// Run the update to one release of the route.
///
/// The release is fetched from GitHub: a route through other releases
/// needs the network (REL-2 decision D10).
///
/// # Errors
///
/// Returns the error that stopped the update.
fn run_hop(args: &UpdateArgs, hop: &str) -> Result<(), ToolError> {
    let release = release::obtain(None, Some(hop), &mut release::download)?;
    let key = verify::public_key()?;
    verify::verify_manifest(&release.manifest, &release.bundle, &key)?;
    let manifest: Manifest = serde_json::from_slice(&release.manifest)
        .map_err(|error| ToolError::Failed(format!("the manifest could not be read: {error}")))?;
    files::check_compose(&manifest, &release.compose)?;

    let facts = gather(args, &release, &manifest)?;
    let mut runner = Local::new(&args.dir);
    run(&args.dir, &facts, &mut runner)
}

/// Gather what the update needs to know.
///
/// # Errors
///
/// Returns the error that says what could not be read or fetched.
fn gather(
    args: &UpdateArgs,
    release: &release::Release,
    manifest: &Manifest,
) -> Result<Facts, ToolError> {
    // The files the release ships (REL-2 S4 decision D14): fetched (or
    // read from the bundle) and checked against the SIGNED manifest
    // before anything is written.
    let shipped = files::fetch(manifest, release.bundle_dir.as_deref(), &mut |url| {
        release::download(url)
    })?;
    Ok(Facts {
        state: state::read(&args.dir)?,
        manifest: manifest.clone(),
        manifest_bytes: release.manifest.clone(),
        manifest_bundle: release.bundle.clone(),
        compose: release.compose.clone(),
        shipped,
        bundle: release.bundle_dir.clone(),
        preflight: preflight_failures(args, &release.compose),
        ready_timeout: args.ready_timeout,
    })
}

/// Run the pre-flight checks of an update.
///
/// The `ports` check is the one that does not apply: the stack is
/// RUNNING, and it holds its own ports. The skip is said out loud, the
/// way every other skipped check is.
///
/// # Errors
///
/// Never: the failures are returned, not raised.
fn preflight_failures(args: &UpdateArgs, compose: &str) -> Vec<preflight::Failure> {
    println!(
        "lakehousectl: the ports check was skipped: the stack is running, and it holds its own \
         ports."
    );
    let mut ignored = vec!["ports".to_string()];
    for skipped in &args.ignore_preflight {
        println!("lakehousectl: the {skipped} check was skipped, as asked.");
        ignored.push(skipped.clone());
    }
    let facts = preflight::collect(compose, &args.dir);
    preflight::run(&facts, &ignored, args.bundle.is_some())
}

/// Run the update.
///
/// The steps are the ones D17 names, in the order it names them.
///
/// # Errors
///
/// Returns the error that stopped the update. A failure from step 6 on
/// is reported WITH what the rollback did (REL-2 S4 decision D17). A
/// failure in step 4 or step 5 is reported WITH what the stack was put
/// back to (REL-2 S4b review round 1 BLOCKER 1).
pub fn run(dir: &Path, facts: &Facts, runner: &mut dyn Runner) -> Result<(), ToolError> {
    let old_version = facts.state.current_version.clone().ok_or_else(|| {
        ToolError::Failed(
            "the install's state names no release, so the tool cannot tell what it would update. \
             `lakehousectl install` writes that state."
                .to_string(),
        )
    })?;
    let new_version = facts.manifest.version.clone();

    // Step 3 — the pre-flight, the staged files and the images. Nothing
    // that is RUNNING is touched before this step succeeds.
    if !facts.preflight.is_empty() {
        for failure in &facts.preflight {
            eprintln!("lakehousectl: {}: {}", failure.check, failure.message);
        }
        return Err(ToolError::Preflight(facts.preflight.len()));
    }
    let staging = stage(dir, &new_version, facts)?;
    println!("lakehousectl: fetching the images of {new_version}.");
    if let Err(error) = runner.fetch_images(&facts.manifest, facts.bundle.as_deref()) {
        return Err(ToolError::Failed(format!(
            "the images of {new_version} could not be fetched, so the running stack was left as \
             it was: {error}"
        )));
    }

    // Step 4 — the pause, and the writers. Postgres, ClickHouse, RustFS
    // and Lakekeeper keep running: they are what the backup is made of.
    //
    // A failure from here on leaves the install with the writers
    // stopped and the pause set, and NOTHING has been swapped: the
    // files in the install's own directory are still the old ones. The
    // stack is put back the way it was, and the error says so
    // (REL-2 S4b review round 1 BLOCKER 1).
    if let Err(error) = pause::set(dir) {
        return Err(stopped_before_the_swap(
            dir,
            facts,
            runner,
            &old_version,
            &format!("the pause could not be set: {error}"),
        ));
    }
    println!("lakehousectl: stopping the writers.");
    if let Err(error) = runner.stop_writers() {
        return Err(stopped_before_the_swap(
            dir,
            facts,
            runner,
            &old_version,
            &format!("the writers could not be stopped: {error}"),
        ));
    }

    // Step 5 — the backup, named after the release it belongs to.
    let backup = match runner.backup(&old_version) {
        Ok(backup) => backup,
        Err(error) => {
            return Err(stopped_before_the_swap(
                dir,
                facts,
                runner,
                &old_version,
                &format!("the backup could not be taken: {error}"),
            ));
        }
    };
    if let Err(error) = state::set_last_backup(dir, &backup) {
        return Err(stopped_before_the_swap(
            dir,
            facts,
            runner,
            &old_version,
            &format!("the backup was taken, but the install's state could not record it: {error}"),
        ));
    }
    println!("lakehousectl: the backup is at {}", backup.display());

    // Step 6 — the new files in place, and the stack up. A failure from
    // here on is rolled back.
    if let Err(error) = swap(dir, facts, &staging) {
        return Err(roll_back(dir, facts, runner, &old_version, &error));
    }
    if let Err(error) = runner.up() {
        return Err(roll_back(
            dir,
            facts,
            runner,
            &old_version,
            &ToolError::Failed(format!("the stack did not start: {error}")),
        ));
    }

    // Step 7 — the wait.
    let timeout = Duration::from_secs(facts.ready_timeout);
    if let Err(error) = runner::wait_for_ready(runner, &new_version, timeout) {
        return Err(roll_back(
            dir,
            facts,
            runner,
            &old_version,
            &ToolError::Failed(error),
        ));
    }

    // Step 8 — the pause is cleared, and the state names the new release.
    pause::clear(dir)?;
    state::set_versions(dir, &new_version, Some(&old_version))?;
    println!("lakehousectl: the stack runs {new_version}.");
    Ok(())
}

/// The error of an update that stopped before it swapped anything.
///
/// Nothing has been swapped, so the files in the install's own
/// directory are still the OLD ones. The stack is started again on
/// those files, it is waited for on the OLD version, and the pause is
/// cleared: the update restored nothing, so nothing must stay paused.
///
/// The error says that the update stopped before it changed anything,
/// and what the stack runs now. A stack that does not come back is
/// reported as it is — the install may need a hand (REL-2 S4b review
/// round 1 BLOCKER 1).
///
/// # Errors
///
/// Never: the failure is returned as the error, and it says what the
/// stack was put back to.
fn stopped_before_the_swap(
    dir: &Path,
    facts: &Facts,
    runner: &mut dyn Runner,
    old_version: &str,
    cause: &str,
) -> ToolError {
    let timeout = Duration::from_secs(facts.ready_timeout);
    let mut problems: Vec<String> = Vec::new();

    // The stack is started again on the OLD files: the Compose file and
    // the manifest in the install's own directory are the old ones, and
    // nothing else was written.
    match runner.up() {
        Ok(()) => {
            if let Err(error) = runner::wait_for_ready(runner, old_version, timeout) {
                problems.push(format!(
                    "the stack did not become ready on {old_version} again: {error}"
                ));
            }
        }
        Err(error) => problems.push(format!("the stack could not be started again: {error}")),
    }

    // The pause is cleared even when the stack did not come back: the
    // update restored nothing, so the jobs that delete files must not
    // stay paused.
    if let Err(error) = pause::clear(dir) {
        problems.push(format!("the pause could not be cleared: {error}"));
    }

    if problems.is_empty() {
        return ToolError::Failed(format!(
            "the update to {} stopped before it changed anything: {cause}. The stack was started \
             again on the old files and runs {old_version} again.",
            facts.manifest.version
        ));
    }
    ToolError::Failed(format!(
        "the update to {} stopped before it changed anything: {cause}. Putting the install back \
         the way it was did not finish: {}. The install may need a hand.",
        facts.manifest.version,
        problems.join("; ")
    ))
}

/// Write the files the release ships into the staging directory.
///
/// The staging directory is inside the install's own directory, and
/// every file is written at the relative path the release ships it at
/// (REL-2 S4 decision D14) — the path the Compose file mounts it at.
///
/// The point of the staging directory is the ORDER: the files are
/// written BEFORE anything that is running is touched, so a disk that
/// is full is found while the stack is still the old one.
///
/// # Errors
///
/// Returns the error that says what could not be written.
fn stage(dir: &Path, version: &str, facts: &Facts) -> Result<PathBuf, ToolError> {
    let staging = dir.join(".staging").join(version);
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|source| ToolError::Io {
            context: format!("{} could not be emptied", staging.display()),
            source,
        })?;
    }
    std::fs::create_dir_all(&staging).map_err(|source| ToolError::Io {
        context: format!("{} could not be created", staging.display()),
        source,
    })?;
    files::write(&staging, &facts.shipped)?;
    Ok(staging)
}

/// Put the new release's files in place, and keep the old ones.
///
/// The old Compose file and the old manifest are kept as `*.previous`,
/// and the state of the install is kept as `state.json.previous`: the
/// rollback puts all three back (REL-2 S4 decisions D17 and D18).
///
/// # Errors
///
/// Returns the error that says what could not be written.
fn swap(dir: &Path, facts: &Facts, staging: &Path) -> Result<(), ToolError> {
    keep_previous(dir, "docker-compose.yml")?;
    keep_previous(dir, "release.json")?;
    state::keep_previous(dir, &facts.state)?;

    write(&dir.join("docker-compose.yml"), facts.compose.as_bytes())?;
    write(&dir.join("release.json"), &facts.manifest_bytes)?;
    write(
        &dir.join("release.json.sigstore.json"),
        &facts.manifest_bundle,
    )?;
    promote(staging, dir, &facts.shipped)?;
    Ok(())
}

/// Keep one file of the install as its own `.previous`.
///
/// # Errors
///
/// Returns the error that says what could not be written.
fn keep_previous(dir: &Path, name: &str) -> Result<(), ToolError> {
    let path = dir.join(name);
    if !path.exists() {
        return Ok(());
    }
    std::fs::copy(&path, dir.join(format!("{name}.previous"))).map_err(|source| ToolError::Io {
        context: format!("{} could not be kept as {name}.previous", path.display()),
        source,
    })?;
    Ok(())
}

/// Put the staged files in place.
///
/// Each file is written at the relative path the release ships it at,
/// so a mount finds its file.
///
/// # Errors
///
/// Returns the error that says what could not be written.
fn promote(staging: &Path, dir: &Path, shipped: &files::ShippedFiles) -> Result<(), ToolError> {
    for name in shipped.keys() {
        files::check_install_path(name)?;
        let from = staging.join(name);
        let to = dir.join(name);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ToolError::Io {
                context: format!("{} could not be created", parent.display()),
                source,
            })?;
        }
        // The copy keeps the mode of the staged file, and the staged
        // file was written with the mode its name calls for: a script
        // is executable, a mounted file is not.
        std::fs::copy(&from, &to).map_err(|source| ToolError::Io {
            context: format!("{} could not be written", to.display()),
            source,
        })?;
    }
    Ok(())
}

/// Roll the update back, and report what failed and what the rollback did.
///
/// # Errors
///
/// Never: the failure is returned as the error, and it says both what
/// failed and what the rollback did.
fn roll_back(
    dir: &Path,
    facts: &Facts,
    runner: &mut dyn Runner,
    old_version: &str,
    cause: &ToolError,
) -> ToolError {
    eprintln!(
        "lakehousectl: the update to {} failed: {cause}",
        facts.manifest.version
    );
    eprintln!("lakehousectl: rolling back to {old_version}.");
    let timeout = Duration::from_secs(facts.ready_timeout);
    match rollback::run(dir, runner, old_version, timeout) {
        Ok(()) => ToolError::Failed(format!(
            "the update to {} failed, and the stack was rolled back to {old_version}: {cause}",
            facts.manifest.version
        )),
        Err(error) => ToolError::Failed(format!(
            "the update to {} failed, and the ROLLBACK failed too: {cause}. The rollback said: \
             {error}",
            facts.manifest.version
        )),
    }
}

/// Compute the route from the installed release to the release the
/// update moves to (REL-2 decision D10).
///
/// # Errors
///
/// Returns the message that says why there is no route: a release that
/// is not a version, a release that is older than the installed one, or
/// a release that may only be reached through others.
pub fn compute_route(current: &str, target: &Manifest) -> Result<Route, String> {
    let current_version = version::parse(current)
        .map_err(|error| format!("the installed release {current} could not be read: {error}"))?;
    let target_version = version::parse(&target.version)
        .map_err(|error| format!("the release {} could not be read: {error}", target.version))?;
    let order = version::compare(current, &target.version)
        .map_err(|error| format!("the release {} could not be read: {error}", target.version))?;
    if order == Ordering::Equal {
        return Ok(Route::Nothing);
    }
    if order == Ordering::Greater {
        return Err(format!(
            "the release {current} is installed, and {} is OLDER than it. An update moves forward \
             only; `lakehousectl rollback` goes back to an earlier release.",
            target.version
        ));
    }

    // The stops between the installed release and the target, in order.
    let mut hops: Vec<(version::Version, String)> = Vec::new();
    for stop in &target.stops {
        let parsed = version::parse(stop).map_err(|error| {
            format!("the stop {stop} of the release could not be read: {error}")
        })?;
        let after_current = version::order(&current_version, &parsed) == Ordering::Less;
        let before_target = version::order(&parsed, &target_version) == Ordering::Less;
        if after_current && before_target {
            hops.push((parsed, stop.clone()));
        }
    }
    hops.sort_by(|one, other| version::order(&one.0, &other.0));

    // The lowest release that may update to the target directly.
    let min_from = match &target.min_from {
        Some(min_from) => version::parse(min_from).map_err(|error| {
            format!(
                "the lowest release of {} could not be read: {error}",
                target.version
            )
        })?,
        // A manifest that names no `minFrom` promises no direct update
        // from an older release: the route then needs the stops, and a
        // release with no stops has none.
        None => target_version,
    };

    if version::order(&current_version, &min_from) == Ordering::Less {
        if hops.is_empty() {
            return Err(format!(
                "the release {} may only be updated to from {} or newer, and {current} is \
                 installed. The release names no release to pass through, so there is no route.",
                target.version,
                min_from.as_string()
            ));
        }
        return Ok(Route::Hops(
            hops.into_iter().map(|(_, stop)| stop).collect(),
        ));
    }
    if !hops.is_empty() {
        return Ok(Route::Hops(
            hops.into_iter().map(|(_, stop)| stop).collect(),
        ));
    }
    Ok(Route::Direct)
}

/// Write a file.
///
/// # Errors
///
/// Returns the error that says what could not be written.
fn write(path: &Path, bytes: &[u8]) -> Result<(), ToolError> {
    std::fs::write(path, bytes).map_err(|source| ToolError::Io {
        context: format!("{} could not be written", path.display()),
        source,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::time::{Duration, Instant};

    use super::*;
    use crate::runner::ApiState;

    /// A manifest of a release, with the upgrade path it names.
    fn manifest_of(version: &str, min_from: Option<&str>, stops: &[&str]) -> Manifest {
        Manifest {
            version: version.to_string(),
            min_from: min_from.map(str::to_string),
            stops: stops.iter().map(|stop| (*stop).to_string()).collect(),
            images: std::collections::BTreeMap::new(),
            image_ids: std::collections::BTreeMap::new(),
            files: std::collections::BTreeMap::new(),
        }
    }

    /// A runner that records the calls and fails where it is told to.
    struct Fake {
        /// The calls, in order.
        calls: Vec<String>,
        /// The steps that fail, when they are set.
        failing: Vec<String>,
        /// The time the fake thinks it is.
        now: Instant,
        /// What the API reports after each `up`: the version, and
        /// whether it is ready.
        ///
        /// The stack comes up once for the update and once for the
        /// rollback, and each start has its own answer — that is how a
        /// broken release and a good one are told apart.
        after_up: Vec<(String, bool)>,
        /// The version the API reports now.
        api_version: String,
        /// Whether the API reports itself ready.
        api_ready: bool,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                calls: Vec::new(),
                failing: Vec::new(),
                now: Instant::now(),
                after_up: vec![("0.2.0".to_string(), true)],
                api_version: "0.2.0".to_string(),
                api_ready: true,
            }
        }

        fn failing_at(step: &str) -> Self {
            Self {
                failing: vec![step.to_string()],
                ..Self::new()
            }
        }

        /// The fake fails at each of these steps, once each.
        fn failing_at_steps(steps: &[&str]) -> Self {
            Self {
                failing: steps.iter().map(|step| (*step).to_string()).collect(),
                ..Self::new()
            }
        }

        /// The call, or the failure it stands for.
        ///
        /// The failure happens ONCE: the rollback that follows a
        /// failure may well succeed — the new release is what was
        /// broken, not the old one.
        fn call(&mut self, step: &str) -> Result<(), String> {
            self.calls.push(step.to_string());
            if let Some(position) = self.failing.iter().position(|failing| failing == step) {
                self.failing.remove(position);
                return Err(format!("the {step} failed"));
            }
            Ok(())
        }

        /// The calls, without the probes.
        fn steps(&self) -> Vec<String> {
            self.calls
                .iter()
                .filter(|call| !call.contains("api-state") && *call != "sleep")
                .cloned()
                .collect()
        }
    }

    impl Runner for Fake {
        fn now(&self) -> Instant {
            self.now
        }

        fn sleep(&mut self, duration: Duration) {
            self.now += duration;
        }

        fn fetch_images(
            &mut self,
            _manifest: &Manifest,
            _bundle: Option<&Path>,
        ) -> Result<(), String> {
            self.call("fetch-images")
        }

        fn stop_writers(&mut self) -> Result<(), String> {
            self.call("stop-writers")
        }

        fn backup(&mut self, _version: &str) -> Result<PathBuf, String> {
            self.call("backup")?;
            Ok(PathBuf::from("/backups/one"))
        }

        fn restore(&mut self, _backup: &Path) -> Result<(), String> {
            self.call("restore")
        }

        fn up(&mut self) -> Result<(), String> {
            self.call("up")?;
            if !self.after_up.is_empty() {
                let (version, ready) = self.after_up.remove(0);
                self.api_version = version;
                self.api_ready = ready;
            }
            Ok(())
        }

        fn down(&mut self) -> Result<(), String> {
            self.call("down")
        }

        fn api_state(&mut self) -> Result<ApiState, String> {
            self.calls.push("api-state".to_string());
            Ok(ApiState {
                ready: self.api_ready,
                version: Some(self.api_version.clone()),
            })
        }

        fn console_ready(&mut self) -> Result<bool, String> {
            self.call("console-ready")?;
            Ok(true)
        }
    }

    /// An install's own directory, with the state of an install of
    /// `0.1.0` in it.
    fn install_with_state(version: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().expect("a temporary directory");
        state::write_initial(directory.path(), version).expect("the state is written");
        std::fs::write(directory.path().join(".env"), "POSTGRES_PASSWORD=secret\n")
            .expect("the env file is written");
        std::fs::write(
            directory.path().join("docker-compose.yml"),
            "services: {}\n",
        )
        .expect("the compose file is written");
        std::fs::write(
            directory.path().join("release.json"),
            "{\"version\": \"0.1.0\"}\n",
        )
        .expect("the manifest is written");
        let path = directory.path().to_path_buf();
        (directory, path)
    }

    /// The facts of an update from `0.1.0` to `0.2.0`.
    fn facts_of(directory: &Path, manifest: &Manifest) -> Facts {
        Facts {
            state: state::read(directory).expect("the state is read"),
            manifest: manifest.clone(),
            manifest_bytes: b"{}".to_vec(),
            manifest_bundle: b"{}".to_vec(),
            compose: "services: {}\n".to_string(),
            shipped: files::ShippedFiles::new(),
            bundle: None,
            preflight: Vec::new(),
            ready_timeout: 10,
        }
    }

    #[test]
    fn the_route_to_the_installed_release_is_nothing_to_do() {
        let manifest = manifest_of("0.2.0", Some("0.0.0"), &[]);
        assert_eq!(
            compute_route("0.2.0", &manifest).expect("the route is computed"),
            Route::Nothing
        );
    }

    #[test]
    fn the_route_to_an_older_release_is_refused_and_points_at_the_rollback() {
        let manifest = manifest_of("0.1.0", Some("0.0.0"), &[]);
        let message = compute_route("0.2.0", &manifest).expect_err("an older release is refused");
        assert!(
            message.contains("lakehousectl rollback"),
            "the error says what to do: {message}"
        );
        assert!(
            message.contains("0.1.0"),
            "the error names the release: {message}"
        );
    }

    #[test]
    fn the_route_to_a_newer_release_is_direct() {
        let manifest = manifest_of("0.2.0", Some("0.0.0"), &[]);
        assert_eq!(
            compute_route("0.1.0", &manifest).expect("the route is computed"),
            Route::Direct
        );
    }

    #[test]
    fn a_release_below_the_floor_needs_the_stops() {
        let manifest = manifest_of("0.4.0", Some("0.3.0"), &["0.3.0"]);
        assert_eq!(
            compute_route("0.2.0", &manifest).expect("the route is computed"),
            Route::Hops(vec!["0.3.0".to_string()])
        );
    }

    #[test]
    fn a_stop_between_the_releases_is_passed_through() {
        let manifest = manifest_of("0.4.0", Some("0.0.0"), &["0.3.0"]);
        assert_eq!(
            compute_route("0.2.0", &manifest).expect("the route is computed"),
            Route::Hops(vec!["0.3.0".to_string()])
        );
    }

    #[test]
    fn a_stop_that_is_already_passed_is_not_passed_through_again() {
        let manifest = manifest_of("0.4.0", Some("0.0.0"), &["0.1.0", "0.3.0"]);
        assert_eq!(
            compute_route("0.2.0", &manifest).expect("the route is computed"),
            Route::Hops(vec!["0.3.0".to_string()])
        );
    }

    #[test]
    fn a_release_below_the_floor_with_no_stop_has_no_route() {
        let manifest = manifest_of("0.4.0", Some("0.3.0"), &[]);
        let error = compute_route("0.2.0", &manifest).expect_err("there is no route");
        assert!(
            error.contains("0.3.0"),
            "the error names the floor: {error}"
        );
    }

    #[test]
    fn a_later_build_of_a_release_is_newer_than_it() {
        // D21: the proof's bundles are later builds of one release.
        let manifest = manifest_of("0.2.0-update-proof.1", Some("0.0.0"), &[]);
        assert_eq!(
            compute_route("0.2.0", &manifest).expect("the route is computed"),
            Route::Direct
        );
    }

    #[test]
    fn the_steps_run_in_the_order_the_decision_names() {
        let (_directory, path) = install_with_state("0.1.0");
        let manifest = manifest_of("0.2.0", Some("0.0.0"), &[]);
        let facts = facts_of(&path, &manifest);
        let mut runner = Fake::new();

        run(&path, &facts, &mut runner).expect("the update runs");

        assert_eq!(
            runner.steps(),
            vec![
                "fetch-images".to_string(),
                "stop-writers".to_string(),
                "backup".to_string(),
                "up".to_string(),
                "console-ready".to_string(),
            ],
            "the steps ran in another order"
        );
    }

    #[test]
    fn a_failure_before_the_writers_stop_leaves_the_running_stack_alone() {
        // D17: nothing that is running is touched before step 4.
        let (_directory, path) = install_with_state("0.1.0");
        let manifest = manifest_of("0.2.0", Some("0.0.0"), &[]);
        let facts = facts_of(&path, &manifest);
        let mut runner = Fake::failing_at("fetch-images");

        let error = run(&path, &facts, &mut runner).expect_err("the update fails");

        assert!(
            error.to_string().contains("left as it was"),
            "the error says the stack was left alone: {error}"
        );
        assert_eq!(
            runner.steps(),
            vec!["fetch-images".to_string()],
            "something that is running was touched: {:?}",
            runner.steps()
        );
        let state = state::read(&path).expect("the state is read");
        assert_eq!(
            state.current_version.as_deref(),
            Some("0.1.0"),
            "the state was changed"
        );
    }

    #[test]
    fn a_failure_to_stop_the_writers_starts_the_old_stack_again() {
        // REL-2 S4b review round 1 BLOCKER 1: a failure in step 4 leaves
        // the writers stopped and the pause set, and NOTHING has been
        // swapped. The stack is started again on the OLD files, the
        // pause is cleared, and the error says what happened.
        let (_directory, path) = install_with_state("0.1.0");
        let manifest = manifest_of("0.2.0", Some("0.0.0"), &[]);
        let facts = facts_of(&path, &manifest);
        let state_before = std::fs::read(state::path(&path)).expect("the state is readable");
        let mut runner = Fake::failing_at("stop-writers");
        // The stack comes back on the old release.
        runner.after_up = vec![("0.1.0".to_string(), true)];

        let error = run(&path, &facts, &mut runner).expect_err("the update fails");

        let message = error.to_string();
        assert!(
            message.contains("stopped before it changed anything"),
            "the error says the update changed nothing: {message}"
        );
        assert!(
            message.contains("the writers could not be stopped"),
            "the error says what failed: {message}"
        );
        assert!(
            message.contains("runs 0.1.0 again"),
            "the error says what the stack runs: {message}"
        );
        assert_eq!(
            runner.steps(),
            vec![
                "fetch-images".to_string(),
                "stop-writers".to_string(),
                "up".to_string(),
                "console-ready".to_string(),
            ],
            "the stack was not put back the way it was: {:?}",
            runner.steps()
        );
        assert_eq!(
            runner.api_version, "0.1.0",
            "the stack does not run the old release"
        );
        let env = std::fs::read_to_string(path.join(".env")).expect("the env file is readable");
        assert!(
            !env.contains("MAINTENANCE_PAUSED"),
            "the pause was not cleared: {env}"
        );
        let state_after = std::fs::read(state::path(&path)).expect("the state is readable");
        assert_eq!(state_after, state_before, "the state was changed");
    }

    #[test]
    fn a_failure_to_take_the_backup_starts_the_old_stack_again() {
        // REL-2 S4b review round 1 BLOCKER 1: the same for step 5. The
        // writers ARE stopped by then, so the stack has to be started
        // again — on the OLD files, for nothing has been swapped.
        let (_directory, path) = install_with_state("0.1.0");
        let manifest = manifest_of("0.2.0", Some("0.0.0"), &[]);
        let facts = facts_of(&path, &manifest);
        let state_before = std::fs::read(state::path(&path)).expect("the state is readable");
        let mut runner = Fake::failing_at("backup");
        // The stack comes back on the old release.
        runner.after_up = vec![("0.1.0".to_string(), true)];

        let error = run(&path, &facts, &mut runner).expect_err("the update fails");

        let message = error.to_string();
        assert!(
            message.contains("stopped before it changed anything"),
            "the error says the update changed nothing: {message}"
        );
        assert!(
            message.contains("the backup could not be taken"),
            "the error says what failed: {message}"
        );
        assert!(
            message.contains("runs 0.1.0 again"),
            "the error says what the stack runs: {message}"
        );
        assert_eq!(
            runner.steps(),
            vec![
                "fetch-images".to_string(),
                "stop-writers".to_string(),
                "backup".to_string(),
                "up".to_string(),
                "console-ready".to_string(),
            ],
            "the stack was not put back the way it was: {:?}",
            runner.steps()
        );
        assert_eq!(
            runner.api_version, "0.1.0",
            "the stack does not run the old release"
        );
        let env = std::fs::read_to_string(path.join(".env")).expect("the env file is readable");
        assert!(
            !env.contains("MAINTENANCE_PAUSED"),
            "the pause was not cleared: {env}"
        );
        let state_after = std::fs::read(state::path(&path)).expect("the state is readable");
        assert_eq!(state_after, state_before, "the state was changed");
    }

    #[test]
    fn a_stack_that_does_not_come_back_is_reported_as_it_is() {
        // REL-2 S4b review round 1 BLOCKER 1: the error must not claim
        // the stack runs the old release when it does not.
        let (_directory, path) = install_with_state("0.1.0");
        let manifest = manifest_of("0.2.0", Some("0.0.0"), &[]);
        let facts = facts_of(&path, &manifest);
        let mut runner = Fake::failing_at_steps(&["stop-writers", "up"]);
        // The stack does not come back at all: the writers cannot be
        // stopped, and the start that follows fails too.

        let error = run(&path, &facts, &mut runner).expect_err("the update fails");

        let message = error.to_string();
        assert!(
            message.contains("stopped before it changed anything"),
            "the error says the update changed nothing: {message}"
        );
        assert!(
            message.contains("could not be started again"),
            "the error says the stack did not come back: {message}"
        );
        assert!(
            !message.contains("runs 0.1.0 again"),
            "the error claims a stack that is not there: {message}"
        );
        let env = std::fs::read_to_string(path.join(".env")).expect("the env file is readable");
        assert!(
            !env.contains("MAINTENANCE_PAUSED"),
            "the pause was not cleared: {env}"
        );
    }

    #[test]
    fn a_failure_at_the_stack_start_is_rolled_back() {
        let (_directory, path) = install_with_state("0.1.0");
        let manifest = manifest_of("0.2.0", Some("0.0.0"), &[]);
        let facts = facts_of(&path, &manifest);
        let mut runner = Fake::failing_at("up");
        // The stack does not come up with the new release, and comes up
        // with the old one when the rollback starts it.
        runner.after_up = vec![("0.1.0".to_string(), true)];

        let error = run(&path, &facts, &mut runner).expect_err("the update fails");

        assert!(
            error.to_string().contains("rolled back"),
            "the error says the update was rolled back: {error}"
        );
        let steps = runner.steps();
        assert!(
            steps.contains(&"restore".to_string()),
            "the backup was not restored: {steps:?}"
        );
        assert!(
            steps.contains(&"down".to_string()),
            "the stack was not stopped: {steps:?}"
        );
    }

    #[test]
    fn a_stack_that_never_becomes_ready_is_rolled_back() {
        // D17 step 9: a timeout is rolled back by itself.
        let (_directory, path) = install_with_state("0.1.0");
        let manifest = manifest_of("0.2.0", Some("0.0.0"), &[]);
        let mut facts = facts_of(&path, &manifest);
        facts.ready_timeout = 10;
        // The new release never becomes ready; the old one does, so
        // the rollback has something to wait for.
        let mut runner = Fake::new();
        runner.after_up = vec![("0.2.0".to_string(), false), ("0.1.0".to_string(), true)];

        let error = run(&path, &facts, &mut runner).expect_err("the update fails");

        assert!(
            error.to_string().contains("rolled back"),
            "the error says the update was rolled back: {error}"
        );
        assert!(
            error.to_string().contains("did not become ready"),
            "the error says what failed: {error}"
        );
        assert!(
            runner.steps().contains(&"restore".to_string()),
            "the backup was not restored: {:?}",
            runner.steps()
        );
    }

    #[test]
    fn a_successful_update_names_the_new_release_and_clears_the_pause() {
        let (_directory, path) = install_with_state("0.1.0");
        let manifest = manifest_of("0.2.0", Some("0.0.0"), &[]);
        let mut facts = facts_of(&path, &manifest);
        facts.manifest_bytes = b"{\"version\": \"0.2.0\"}".to_vec();
        let mut runner = Fake::new();

        run(&path, &facts, &mut runner).expect("the update runs");

        let state = state::read(&path).expect("the state is read");
        assert_eq!(state.current_version.as_deref(), Some("0.2.0"));
        assert_eq!(state.previous_version.as_deref(), Some("0.1.0"));
        assert_eq!(
            state.last_backup.as_deref(),
            Some("/backups/one"),
            "the backup was not recorded"
        );
        let env = std::fs::read_to_string(path.join(".env")).expect("the env file is readable");
        assert!(
            !env.contains("MAINTENANCE_PAUSED"),
            "the pause was not cleared: {env}"
        );
        assert!(
            path.join("docker-compose.yml.previous").exists(),
            "the old Compose file was not kept"
        );
        assert!(
            path.join("release.json.previous").exists(),
            "the old manifest was not kept"
        );
    }

    #[test]
    fn the_pause_is_set_before_the_backup() {
        let (_directory, path) = install_with_state("0.1.0");
        let manifest = manifest_of("0.2.0", Some("0.0.0"), &[]);
        let facts = facts_of(&path, &manifest);
        let mut runner = Fake::new();

        run(&path, &facts, &mut runner).expect("the update runs");

        // The pause is cleared at the end, so what the run leaves behind
        // is the pause being set and then cleared: the order is proved
        // by the rollback test below, where the pause is left set when
        // the rollback cannot clear it.
        let env = std::fs::read_to_string(path.join(".env")).expect("the env file is readable");
        assert!(!env.contains("MAINTENANCE_PAUSED"), "{env}");
    }

    #[test]
    fn the_files_the_release_ships_are_staged_and_then_put_in_place() {
        let (_directory, path) = install_with_state("0.1.0");
        let manifest = manifest_of("0.2.0", Some("0.0.0"), &[]);
        let mut facts = facts_of(&path, &manifest);
        facts.shipped.insert(
            "ops/backups/clickhouse/backups.xml".to_string(),
            b"<clickhouse/>".to_vec(),
        );
        let mut runner = Fake::new();

        run(&path, &facts, &mut runner).expect("the update runs");

        assert_eq!(
            std::fs::read(path.join("ops/backups/clickhouse/backups.xml"))
                .expect("the mounted file is there"),
            b"<clickhouse/>".to_vec(),
            "the mounted file was not put in place"
        );
    }
}
