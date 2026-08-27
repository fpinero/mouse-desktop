# winget manifests

The manifests that make `winget install fpinero.mouse-desktop` work, kept here so they are
versioned alongside the release they describe rather than living only in a fork of
`microsoft/winget-pkgs` that gets deleted after a merge.

One directory per version. Each holds the three files a multi-file manifest needs:

```
0.1.0/
  fpinero.mouse-desktop.yaml              version manifest
  fpinero.mouse-desktop.installer.yaml    URLs, hashes, install type
  fpinero.mouse-desktop.locale.en-US.yaml description, tags, links
```

## Submitting a version

1. Fork <https://github.com/microsoft/winget-pkgs> if you have not already.
2. Copy the version directory into
   `manifests/f/fpinero/mouse-desktop/<version>/`. The path is not free form: it is
   `manifests/<first letter of the publisher, lower case>/<Publisher>/<PackageName>/<version>/`
   and it has to agree with `PackageIdentifier`.
3. On a Windows machine, check it the way the pipeline will:

   ```powershell
   winget validate --manifest .\manifests\f\fpinero\mouse-desktop\0.1.0\
   winget install --manifest .\manifests\f\fpinero\mouse-desktop\0.1.0\
   ```

   The second one really installs it. Do that before opening the pull request, not after.
4. Open the pull request from your fork.

## The risk, stated up front

The validation pipeline of `winget-pkgs` distrusts executables that are unsigned and have no
reputation, which is exactly what this is. A rejection is a realistic outcome and not a sign
that anything here is wrong.

If it happens, do not push back on it. Note it, and try again when the binary has an
Authenticode signature or enough age and prevalence to clear the check on its own. Signing is
the only thing that fixes this properly, and it fixes the Attack Surface Reduction block at
the same time.

Worth being clear about something else: **being on winget does not unblock anybody**. A
managed machine that refuses to run this file refuses it whatever installed it, because the
block applies to the file and not to how it arrived. These channels exist to accumulate real
installs, which is what feeds prevalence over time.

## What to change when a new version ships

Copy the newest directory to the new version number and update, in
`fpinero.mouse-desktop.installer.yaml`:

- `PackageVersion` in all three files
- `ReleaseDate`
- the two `InstallerUrl` values, which carry the tag
- the two `InstallerSha256` values

Then run `./validate.sh <version>` from this directory, which checks the files against the
published schemas and re-computes both hashes from the real downloads.

## Two decisions in the installer manifest

**`InstallerType: portable`, not `zip`.** The release publishes a bare `.exe`, so there is no
archive to unpack. Most portable packages on winget are zips and their manifests look
different for that reason.

**`Commands: [mouse-desktop]`.** Without it, winget derives the alias from the file name and
you would start it by typing `mouse-desktop-x64`. Every piece of documentation, the
troubleshooting table and `Get-Process` all say `mouse-desktop`. The Scoop manifest solves
the same problem with a `#/mouse-desktop.exe` fragment on the URL.

**Versioned URLs, never `releases/latest/download/...`.** A manifest pins one version and its
hash. A URL that follows the newest release would stop matching its own hash the day a new
one ships.
