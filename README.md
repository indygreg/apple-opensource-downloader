# Apple Open Source Downloader

This repository defines a Rust crate and CLI program to automate the downloading
of Apple's open source code from https://opensource.apple.com/.

The primary goal of this project is to enable more intuitive usage and
inspection of Apple's open source code. Using this tool you can:

* Convert the history of an Apple component (like the `xnu` core OS
  primitives) to a Git repository and easily view differences between releases.
* Convert the history of a set of open source components (such as everything
  comprising macOS) to a Git repository and easily view differences between
  releases.
* Query for all available open source components and releases.

The canonical home for this project is
https://github.com/indygreg/apple-opensource-downloader. Please report issues or
submit enhancements there.

# Installing

```
# From crates.io
$ cargo install apple-opensource-downloader

# From Git
$ cargo install --git https://github.com/indygreg/apple-opensource-downloader.git --branch main
```

# Using

The `apple-opensource-downloader` CLI is provided. It defines sub-commands to
perform various actions. Run `apple-opensource-downloader help` to see the help.

## Download a Single Component to a Git Repository

The `component-to-git` sub-command will download all versions of a named
software component (see the component list at
https://opensource.apple.com/tarballs/) and write their contents as Git commits
to a Git repository. This allows you to see differences between the versions of
a component.

```
$ apple-opensource-downloader component-to-git --no-bare xnu aos/xnu
<wait for this to finish>

$ cd aos/xnu
$ git log
commit c2011455c3d75195791bd20d189abae4917c8c81 (HEAD -> main, tag: 7195.141.2)
Author: Apple Open Source <opensource@apple.com>
Date:   Fri Jan 1 00:00:00 2021 +0000

    xnu 7195.141.2

    Downloaded from https://opensource.apple.com/tarballs/xnu/xnu-7195.141.2.tar.gz

commit e76ea20b5519ae2eaf2b74698bd2331141e028fa (tag: 7195.121.3)
Author: Apple Open Source <opensource@apple.com>
Date:   Fri Jan 1 00:00:00 2021 +0000

    xnu 7195.121.3

    Downloaded from https://opensource.apple.com/tarballs/xnu/xnu-7195.121.3.tar.gz

...
```

The Git trees and commit objects should be deterministic provided that the
version of this software is identical and the Apple-hosted source archives don't
change. i.e. different machines should produce Git commits with the same
commit IDs.

## Download all Components to Git Repositories

The `components-to-gits` sub-command will download each available component and
write each to separate Git repositories. It is equivalent to running
`component-to-git` for every named component.

## Download An Apple Software Release to a Git Repository

The `release-to-git` command can be used to download all components in a logical
Apple software release (such as macOS or iOS) to a Git repository. This enables
you to view the history and changes of open source components between releases.

```
$ apple-opensource-downloader release-to-git --no-bare macos aos/macOS
$ cd aos/macOS
$ git log


```

# Known Issues

The HTML parsing isn't the most robust and may not scrape all available software.

If Apple changes the HTML on opensource.apple.com, it will break this tool.

If Apple imposes throttling on their servers, it will likely break this tool.

Created Git repositories don't use packfiles and their performance may be
sub-optimal. Run `git gc` after Git repo creation to optimize the Git repositories.

We don't support incrementally updating Git repositories. Git repositories have
their history recreated from scratch on every invocation. This is obviously
inefficient.

Various advertised URLs on opensource.apple.com result in an HTTP 404. These
are sometimes ignored by this tool.

When importing software releases (such as macOS), the components from one release
to the next may vary. e.g. SQLite could be there in release A, gone in release B,
and reappear in release C. This may make `git diff` output non-representative.

Git commits have a hard-coded date that has no basis in reality.

The naming and layout of Apple's components can at times be confusing and
inconsistent. We don't yet make an effort to reconcile this.

# Legal Compliance

The content downloaded by this tool may be governed by license and usage
restrictions defined outside this tool. Check for usage restrictions
posted at https://opensource.apple.com/ and within the downloaded content.

i.e. if you redistribute the downloaded content, Apple may take an issue
with that.
