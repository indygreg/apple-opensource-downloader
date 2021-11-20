// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use {
    anyhow::{anyhow, Result},
    clap::{App, AppSettings, Arg, SubCommand},
    std::path::Path,
};

pub mod download;
pub mod git;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let app = App::new("Apple Open Source Downloader")
        .setting(AppSettings::ArgRequiredElseHelp)
        .version("0.1")
        .author("Gregory Szorc <gregory.szorc@gmail.com>")
        .about("Download Apple open source code");

    let app = app
        .subcommand(SubCommand::with_name("components").about("Print available component names"));

    let app = app.subcommand(
        SubCommand::with_name("component-versions")
            .about("Print available versions of a given component")
            .arg(
                Arg::with_name("component")
                    .multiple(true)
                    .help("component name"),
            ),
    );

    let app = app.subcommand(
        SubCommand::with_name("component-to-git")
            .about("Fetch an Apple open source component and convert to a Git repository")
            .arg(
                Arg::with_name("no_bare")
                    .long("--no-bare")
                    .help("Do not create a bare repository"),
            )
            .arg(
                Arg::with_name("component")
                    .required(true)
                    .help("component name"),
            )
            .arg(
                Arg::with_name("dest")
                    .required(true)
                    .help("Destination directory of Git repository"),
            ),
    );

    let app = app.subcommand(
        SubCommand::with_name("components-to-gits")
            .about("Fetch Apple open source components and convert to Git repositories")
            .arg(
                Arg::with_name("no_bare")
                    .long("--no-bare")
                    .help("Do not create bare Git repositories)"),
            )
            .arg(
                Arg::with_name("dest")
                    .required(true)
                    .help("Destination directory for Git repositories"),
            ),
    );

    let app = app
        .subcommand(SubCommand::with_name("releases").about("Print available software releases"));

    let app = app.subcommand(
        SubCommand::with_name("release-components")
            .about("Print available components within a software release")
            .arg(
                Arg::with_name("release")
                    .required(true)
                    .help("Name of software release"),
            )
            .arg(
                Arg::with_name("version")
                    .required(true)
                    .help("Version of software release"),
            ),
    );

    let app = app.subcommand(
        SubCommand::with_name("release-to-git")
            .about("Convert a released entity to a Git repository")
            .arg(
                Arg::with_name("no_bare")
                    .long("--no-bare")
                    .help("Do not create a bare repository"),
            )
            .arg(
                Arg::with_name("release")
                    .required(true)
                    .help("Name of released entity"),
            )
            .arg(
                Arg::with_name("dest")
                    .required(true)
                    .help("Destination directory of Git repository"),
            ),
    );

    let matches = app.get_matches();

    match matches.subcommand() {
        ("components", _) => {
            let downloader = crate::download::Downloader::new()?;

            for component in downloader.get_components().await? {
                println!("{}", component);
            }

            Ok(())
        }

        ("component-versions", Some(args)) => {
            let downloader = crate::download::Downloader::new()?;

            if let Some(components) = args.values_of("component") {
                for component in components {
                    for record in downloader.get_component_versions(component).await? {
                        println!("{}\t{}\t{}", record.component, record.version, record.url);
                    }
                }
            } else {
                for records in downloader.get_components_versions().await?.values() {
                    for record in records {
                        println!("{}\t{}\t{}", record.component, record.version, record.url);
                    }
                }
            }

            Ok(())
        }

        ("component-to-git", Some(args)) => {
            let bare = !args.is_present("no_bare");
            let component = args
                .value_of("component")
                .expect("component argument is required");
            let dest = Path::new(args.value_of_os("dest").expect("dest argument is required"));

            crate::git::create_component_repository(dest, component, bare).await
        }

        ("components-to-gits", Some(args)) => {
            let bare = !args.is_present("no_bare");
            let dest = Path::new(args.value_of_os("dest").expect("dest argument is required"));

            crate::git::create_components_repositories(dest, bare).await
        }

        ("releases", _) => {
            let downloader = crate::download::Downloader::new()?;

            for record in downloader.get_releases().await? {
                println!("{}\t{}", record.entity, record.version);
            }

            Ok(())
        }

        ("release-components", Some(args)) => {
            let release = args
                .value_of("release")
                .expect("release argument is required");
            let version = args
                .value_of("version")
                .expect("version argument is required");

            let downloader = crate::download::Downloader::new()?;

            let record = downloader
                .get_releases()
                .await?
                .into_iter()
                .find(|record| record.entity == release && record.version == version)
                .ok_or_else(|| anyhow!("failed to find version {} of {}", version, release))?;

            for component in downloader.get_release_components(&record).await? {
                println!("{}\t{}", component.component, component.url);
            }

            Ok(())
        }

        ("release-to-git", Some(args)) => {
            let bare = !args.is_present("no_bare");
            let release = args
                .value_of("release")
                .expect("release argument is required");
            let dest = Path::new(args.value_of_os("dest").expect("dest argument is required"));

            crate::git::create_release_repository(dest, release, bare).await
        }

        _ => Err(anyhow!("invalid sub-command")),
    }
}
