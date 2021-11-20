// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use {
    crate::download::{Downloader, ReleaseComponentRecord},
    anyhow::{anyhow, Context, Result},
    async_compression::tokio::bufread::GzipDecoder,
    futures::StreamExt,
    git2::{Commit, Oid, Repository, RepositoryInitOptions, Signature, TreeBuilder},
    std::{collections::HashMap, io::Cursor, path::Path, pin::Pin},
    tokio::io::AsyncReadExt,
    tokio_tar::Archive,
};

const GIT_TREE_MODE: i32 = 0o40000;

/// Write content in a tar archive to a Git repository.
///
/// Returns the Git tree Oid.
pub async fn tar_data_to_tree(tar_data: &[u8], repo: &Repository) -> Result<Oid> {
    let reader = GzipDecoder::new(Cursor::new(tar_data));

    let mut archive = Archive::new(reader);

    let mut dirs: HashMap<Vec<u8>, TreeBuilder> = HashMap::new();

    let mut entries = archive.entries().context("reading tar entries")?;

    let mut pinned = Pin::new(&mut entries);
    while let Some(entry) = pinned.next().await {
        let mut entry = entry.context("reading tar entry")?;

        if entry.header().entry_type().is_dir() {
            continue;
        }

        let original_mode = entry.header().mode()? as i32;

        let (mode, buf) = if let Some(link_name) = entry.header().link_name_bytes() {
            (0o120000, link_name.to_vec())
        } else {
            let mode = if original_mode & 0o111 != 0 {
                0o100755
            } else if original_mode & 0o444 != 0 {
                0o100644
            // This occurs in some archives.
            } else if original_mode == 0 {
                0o100644
            } else {
                return Err(anyhow!("invalid tar archive mode: {}", original_mode));
            };

            let mut buf = vec![];
            entry.read_to_end(&mut buf).await?;

            (mode, buf)
        };

        let blob_oid = repo.blob(&buf).context("writing file data to blob")?;

        let path = entry.path_bytes();

        // First directory is ignored.
        let start_index = if let Some(i) =
            path.iter()
                .enumerate()
                .find_map(|(index, c)| if *c == b'/' { Some(index) } else { None })
        {
            i
        } else {
            println!(
                "ignoring tar member {} not in sub-directory",
                String::from_utf8_lossy(&path)
            );
            continue;
        };

        let path = &path[start_index + 1..];

        let (dir, filename) = if let Some(dir_index) = path
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, c)| if *c == b'/' { Some(index) } else { None })
        {
            let dir = &path[..dir_index];
            let filename = &path[dir_index + 1..];

            (dir.to_vec(), filename.to_vec())
        } else {
            (vec![], path.to_vec())
        };

        // Ensure parent directories have treebuilders.
        for (i, c) in dir.iter().enumerate() {
            if *c == b'/' {
                let parent = &dir[..i];

                dirs.entry(Vec::from(parent))
                    .or_insert_with(|| repo.treebuilder(None).unwrap());
            }
        }

        dirs.entry(dir)
            .or_insert_with(|| repo.treebuilder(None).unwrap())
            .insert(filename, blob_oid, mode)?;
    }

    // Ensure root is present, since it is special.
    dirs.entry(vec![])
        .or_insert_with(|| repo.treebuilder(None).unwrap());

    // dirs now holds each logical directory/tree and its files. We need to walk from
    // the child-most nodes down to the root to write the tree objects and populate
    // parents with the just-written tree object.
    let mut keys = dirs.keys().cloned().collect::<Vec<_>>();
    keys.sort_by(|a, b| b.len().cmp(&a.len()));

    for key in &keys {
        // Finalize this tree.
        let tree = dirs.get(key).expect("iterating over known keys");
        let oid = tree.write()?;

        // Record just-written tree in parent if not at root.
        if let Some(end_index) =
            key.iter()
                .enumerate()
                .rev()
                .find_map(|(index, c)| if *c == b'/' { Some(index) } else { None })
        {
            let parent_path = &key[0..end_index];
            let tree_path = &key[end_index + 1..];

            dirs.get_mut(parent_path)
                .expect("parent directory should always be present")
                .insert(tree_path, oid, GIT_TREE_MODE)?;
        } else if !key.is_empty() {
            dirs.get_mut(&vec![])
                .expect("root directory should always be present")
                .insert(&key, oid, GIT_TREE_MODE)?;
        } else {
            return Ok(oid);
        }
    }

    panic!("should have emitted root tree in loop above");
}

pub fn reconcile_repo_to_commit(
    repo: &Repository,
    branch_name: &str,
    commit: &Commit,
) -> Result<()> {
    if repo.is_bare() {
        repo.branch(branch_name, commit, true)
            .context("updating branch to commit")?;
    } else {
        repo.set_head_detached(commit.id())
            .context("marking head as detached")?;
        repo.branch(branch_name, commit, true)
            .context("updating branch to commit")?;
        repo.set_head(&format!("refs/heads/{}", branch_name))
            .context("setting head to branch")?;
        repo.reset(commit.as_object(), git2::ResetType::Hard, None)
            .context("resetting working directory")?;
    }

    Ok(())
}

/// Create a Git repository for an Apple opensource component.
///
/// The Git repository will have tags corresponding to the versions of the component.
pub async fn create_component_repository(
    path: impl AsRef<Path>,
    component: &str,
    bare: bool,
) -> Result<()> {
    let downloader = Downloader::new().context("creating downloader")?;

    let records = downloader
        .get_component_versions(component)
        .await
        .context("fetching component versions")?;

    let branch_name = "main";

    let repo = Repository::init_opts(
        path,
        RepositoryInitOptions::new()
            .bare(bare)
            .initial_head(branch_name),
    )
    .context("initialing repository")?;

    let mut parent_commit = None;

    let signature = Signature::new(
        "Apple Open Source",
        "opensource@apple.com",
        &git2::Time::new(1609459200, 0),
    )?;

    for record in records {
        let tar_data = downloader
            .get_component_record(&record)
            .await
            .context("fetching component tarball")?;

        let tree_oid = tar_data_to_tree(&tar_data, &repo).await?;
        let tree = repo.find_tree(tree_oid)?;

        let parents = if let Some(parent) = &parent_commit {
            vec![parent]
        } else {
            vec![]
        };

        let commit_oid = repo.commit(
            None,
            &signature,
            &signature,
            &format!(
                "{} {}\n\nDownloaded from {}\n",
                record.component, record.version, record.url
            ),
            &tree,
            &parents,
        )?;

        println!(
            "Committed {} version {} as {}",
            record.component, record.version, commit_oid
        );

        let commit = repo.find_commit(commit_oid)?;

        repo.tag(
            &record.version,
            commit.as_object(),
            &signature,
            "tagging",
            true,
        )?;

        parent_commit = Some(commit);
    }

    if let Some(parent) = parent_commit {
        reconcile_repo_to_commit(&repo, branch_name, &parent)?;
    }

    Ok(())
}

pub async fn create_components_repositories(path: &Path, bare: bool) -> Result<()> {
    let downloader = Downloader::new().context("creating downloader")?;

    let components = downloader
        .get_components()
        .await
        .context("resolving components")?;

    let mut errors = vec![];

    for fs in futures::future::join_all(
        components
            .iter()
            .map(|c| create_component_repository(path.join(c), c, bare)),
    )
    .await
    {
        if let Err(e) = fs {
            errors.push(e);
        }
    }

    for err in errors {
        println!("{:?}", err);
    }

    Ok(())
}

async fn import_release_component(
    downloader: &Downloader,
    repo: &Repository,
    component: ReleaseComponentRecord,
) -> Result<Option<(ReleaseComponentRecord, Oid)>> {
    let tar_data = match downloader
        .get_release_component_record(&component)
        .await
        .context("fetching release component record")
    {
        Ok(x) => x,
        Err(e) => {
            println!(
                "warning: {} failed to download; skipping ({:?})",
                component.url, e
            );
            return Ok(None);
        }
    };

    let tree_oid = tar_data_to_tree(&tar_data, &repo)
        .await
        .with_context(|| format!("converting {} to Git tree", component.url))?;

    println!("imported {} to Git", component.url);

    Ok(Some((component, tree_oid)))
}

pub async fn create_release_repository(path: &Path, release: &str, bare: bool) -> Result<()> {
    let downloader = Downloader::new().context("creating downloader")?;

    let branch_name = "main";

    let repo = Repository::init_opts(
        path,
        RepositoryInitOptions::new()
            .bare(bare)
            .initial_head(branch_name),
    )
    .context("initialing repository")?;

    let mut seen_trees: HashMap<String, Oid> = HashMap::new();

    let mut parent_commit = None;

    for record in downloader
        .get_releases()
        .await
        .context("fetching releases")?
        .into_iter()
        .filter(|record| record.matches_entity(release))
    {
        println!("building commit for {} {}", record.entity, record.version);

        let components = downloader
            .get_release_components(&record)
            .await
            .with_context(|| {
                format!(
                    "fetching components for release {} {}",
                    record.entity, record.version
                )
            })?;

        let mut root_builder = repo.treebuilder(None).context("creating tree builder")?;

        let signature = Signature::new(
            "Apple Open Source",
            "opensource@apple.com",
            &git2::Time::new(1609459200, 0),
        )?;

        let mut missing = vec![];

        for component in components {
            if let Some(tree_oid) = seen_trees.get(&component.url) {
                println!("using already imported archive {}", component.url);
                root_builder.insert(&component.component, tree_oid.clone(), GIT_TREE_MODE)?;
            } else {
                missing.push(component);
            }
        }

        for fs in futures::future::join_all(
            missing
                .into_iter()
                .map(|component| import_release_component(&downloader, &repo, component)),
        )
        .await
        {
            if let Some((component, tree_oid)) = fs? {
                seen_trees.insert(component.url, tree_oid.clone());
                root_builder.insert(component.component, tree_oid, GIT_TREE_MODE)?;
            }
        }

        let tree_oid = root_builder.write().context("writing root tree object")?;
        let tree = repo.find_tree(tree_oid)?;

        let parents = if let Some(parent) = &parent_commit {
            vec![parent]
        } else {
            vec![]
        };

        let commit_oid = repo.commit(
            None,
            &signature,
            &signature,
            &format!("{} {}", record.entity, record.version),
            &tree,
            &parents,
        )?;

        println!(
            "Committed {} version {} as {}",
            record.entity, record.version, commit_oid
        );

        let commit = repo.find_commit(commit_oid)?;

        repo.tag(
            &record.version,
            commit.as_object(),
            &signature,
            "tagging",
            true,
        )?;

        parent_commit = Some(commit);
    }

    if let Some(parent) = parent_commit {
        reconcile_repo_to_commit(&repo, branch_name, &parent)?;
    }

    Ok(())
}
