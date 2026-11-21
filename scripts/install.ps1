<#
.SYNOPSIS
    Install RIONT on Windows and add it to your User PATH.

.DESCRIPTION
    Downloads the latest Windows release archive from GitHub Releases (or a
    specific -Version / local -ArchivePath), verifies its SHA256 checksum when
    available, installs riont.exe to %LOCALAPPDATA%\riont\bin by default, and
    adds that folder to your User PATH so `riont` works from any new terminal.

.EXAMPLE
    irm https://raw.githubusercontent.com/lukalyc/RIoNT/master/scripts/install.ps1 | iex
#>
param(
    [string]$Version = "",
    [string]$InstallDir = "",
    [string]$ArchivePath = "",
    [switch]$NoModifyPath
)

& {
    param(
        [string]$VersionParam,
        [string]$InstallDirParam,
        [string]$ArchivePathParam,
        [bool]$NoModifyPathParam
    )

    $ErrorActionPreference = "Stop"
    $ProgressPreference = "SilentlyContinue"
    [Net.ServicePointManager]::SecurityProtocol = (
        [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    )

    $repoUrl = "https://github.com/lukalyc/RIoNT"

    $ver = if (-not [string]::IsNullOrWhiteSpace($VersionParam)) { $VersionParam } else { $env:RIONT_VERSION }
    $destDir = if (-not [string]::IsNullOrWhiteSpace($InstallDirParam)) { $InstallDirParam } else { $env:RIONT_INSTALL_DIR }
    $localArchive = if (-not [string]::IsNullOrWhiteSpace($ArchivePathParam)) { $ArchivePathParam } else { $env:RIONT_ARCHIVE_PATH }
    $skipPath = $NoModifyPathParam -or ($env:RIONT_NO_MODIFY_PATH -eq "1")

    if ([string]::IsNullOrWhiteSpace($destDir)) {
        if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
            throw "LOCALAPPDATA is not set. Pass -InstallDir explicitly."
        }
        $destDir = Join-Path $env:LOCALAPPDATA "riont\bin"
    }

    $arch = $env:PROCESSOR_ARCHITEW6432
    if ([string]::IsNullOrWhiteSpace($arch)) {
        $arch = $env:PROCESSOR_ARCHITECTURE
    }
    if ($arch -eq "AMD64") {
        $target = "x86_64-pc-windows-msvc"
    } elseif ($arch -eq "ARM64") {
        Write-Host "Note: ARM64 Windows detected; installing x64 build (runs via Windows x64 emulation)."
        $target = "x86_64-pc-windows-msvc"
    } else {
        throw "Unsupported Windows architecture '$arch'. RIONT publishes 64-bit (x86_64) Windows binaries."
    }

    function Resolve-LatestVersion {
        $url = "$repoUrl/releases/latest"
        for ($i = 0; $i -lt 5; $i++) {
            $req = [System.Net.HttpWebRequest]::Create($url)
            $req.AllowAutoRedirect = $false
            $req.UserAgent = "riont-installer"
            $resp = $req.GetResponse()
            try {
                $status = [int]$resp.StatusCode
                if ($status -ge 300 -and $status -lt 400) {
                    $loc = $resp.Headers["Location"]
                    if ([string]::IsNullOrWhiteSpace($loc)) { break }
                    if ($loc -match '/releases/tag/v?([^/?#]+)$') {
                        return $Matches[1]
                    }
                    if ($loc -notmatch '^https?://') {
                        $baseUri = [System.Uri]::new($url)
                        $url = ([System.Uri]::new($baseUri, $loc)).AbsoluteUri
                    } else {
                        $url = $loc
                    }
                } else {
                    if ($resp.ResponseUri -and $resp.ResponseUri.AbsoluteUri -match '/releases/tag/v?([^/?#]+)$') {
                        return $Matches[1]
                    }
                    break
                }
            } finally {
                $resp.Close()
            }
        }
        throw "Could not determine the latest RIONT release from GitHub. Pass -Version (e.g. 0.9.1) or check your network connection."
    }

    function Add-ToUserPath {
        param([string]$DirPath)

        $normTarget = $DirPath.TrimEnd('\', '/')
        $envKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment", $true)
        if ($null -eq $envKey) {
            $envKey = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey("Environment")
        }
        $added = $false
        try {
            $rawPath = $envKey.GetValue(
                "Path",
                "",
                [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames
            )
            if ($null -eq $rawPath) { $rawPath = "" }

            $alreadyPresent = $false
            foreach ($entry in ($rawPath -split ';')) {
                if ([string]::IsNullOrWhiteSpace($entry)) { continue }
                $trimmed = $entry.Trim().TrimEnd('\', '/')
                $expanded = [System.Environment]::ExpandEnvironmentVariables($trimmed).TrimEnd('\', '/')
                if ($expanded.Equals($normTarget, [System.StringComparison]::OrdinalIgnoreCase) -or
                    $trimmed.Equals($normTarget, [System.StringComparison]::OrdinalIgnoreCase)) {
                    $alreadyPresent = $true
                    break
                }
            }

            if (-not $alreadyPresent) {
                $newRawPath = if ([string]::IsNullOrWhiteSpace($rawPath)) {
                    $DirPath
                } elseif ($rawPath.TrimEnd().EndsWith(';')) {
                    "$rawPath$DirPath"
                } else {
                    "$rawPath;$DirPath"
                }
                $envKey.SetValue("Path", $newRawPath, [Microsoft.Win32.RegistryValueKind]::ExpandString)
                $added = $true
            }
        } finally {
            $envKey.Close()
        }

        if ($added) {
            try {
                if (-not ("RiontNative.Win32" -as [type])) {
                    Add-Type -Namespace "RiontNative" -Name "Win32" -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError = true, CharSet = System.Runtime.InteropServices.CharSet.Auto)]
public static extern System.IntPtr SendMessageTimeout(
    System.IntPtr hWnd, uint Msg, System.UIntPtr wParam, string lParam,
    uint fuFlags, uint uTimeout, out System.UIntPtr lpdwResult);
'@
                }
                $result = [System.UIntPtr]::Zero
                [void][RiontNative.Win32]::SendMessageTimeout(
                    [System.IntPtr]0xFFFF,
                    0x001A,
                    [System.UIntPtr]::Zero,
                    "Environment",
                    2,
                    5000,
                    [ref]$result
                )
            } catch {
                # Registry update succeeded; new logins/terminals will still read User PATH.
            }
            Write-Host "Added $DirPath to your User PATH."
        } else {
            Write-Host "$DirPath is already in your User PATH."
        }
    }

    $tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("riont-install-" + [System.Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

    try {
        $zipPath = ""
        if (-not [string]::IsNullOrWhiteSpace($localArchive)) {
            if (-not (Test-Path -LiteralPath $localArchive)) {
                throw "Archive not found: $localArchive"
            }
            $zipPath = (Resolve-Path -LiteralPath $localArchive).Path
            Write-Host "Installing RIONT from local archive: $zipPath"
        } else {
            if ([string]::IsNullOrWhiteSpace($ver)) {
                Write-Host "Checking latest RIONT release..."
                $ver = Resolve-LatestVersion
            }
            $ver = $ver.TrimStart('v')
            $assetName = "riont-$ver-$target.zip"
            $assetUrl = "$repoUrl/releases/download/v$ver/$assetName"
            $sumsUrl = "$repoUrl/releases/download/v$ver/SHA256SUMS.txt"

            $zipPath = Join-Path $tempDir $assetName
            Write-Host "Downloading $assetName (v$ver)..."
            Invoke-WebRequest -Uri $assetUrl -OutFile $zipPath -UseBasicParsing

            $sumsPath = Join-Path $tempDir "SHA256SUMS.txt"
            $sumsDownloaded = $false
            try {
                Invoke-WebRequest -Uri $sumsUrl -OutFile $sumsPath -UseBasicParsing
                $sumsDownloaded = $true
            } catch {
                $statusCode = $null
                if ($_.Exception.Response -and $_.Exception.Response.StatusCode) {
                    $statusCode = [int]$_.Exception.Response.StatusCode
                }
                if ($statusCode -ne 404) {
                    Write-Warning "Could not download SHA256SUMS.txt ($($_.Exception.Message)); skipping checksum verification."
                }
            }

            if ($sumsDownloaded -and (Test-Path -LiteralPath $sumsPath)) {
                $expectedHash = $null
                foreach ($line in (Get-Content -LiteralPath $sumsPath)) {
                    if ($line -match '^\s*([0-9a-fA-F]{64})\s+\*?(.+?)\s*$') {
                        if ($Matches[2] -eq $assetName) {
                            $expectedHash = $Matches[1].ToLowerInvariant()
                            break
                        }
                    }
                }
                if ($expectedHash) {
                    $actualHash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
                    if ($actualHash -ne $expectedHash) {
                        throw "SHA256 checksum mismatch for $assetName (expected $expectedHash, got $actualHash)."
                    }
                    Write-Host "Verified SHA256 checksum."
                }
            }
        }

        $extractDir = Join-Path $tempDir "extract"
        New-Item -ItemType Directory -Path $extractDir -Force | Out-Null
        Expand-Archive -LiteralPath $zipPath -DestinationPath $extractDir -Force

        $exe = Get-ChildItem -Path $extractDir -Recurse -Filter "riont.exe" -File | Select-Object -First 1
        if ($null -eq $exe) {
            throw "riont.exe was not found inside '$zipPath'."
        }

        New-Item -ItemType Directory -Path $destDir -Force | Out-Null
        $destExe = Join-Path $destDir "riont.exe"
        try {
            Copy-Item -LiteralPath $exe.FullName -Destination $destExe -Force
        } catch {
            throw "Failed to write $destExe ($($_.Exception.Message)). Close any running riont terminals and try again."
        }

        if (-not $skipPath) {
            Add-ToUserPath -DirPath $destDir
            $normTarget = $destDir.TrimEnd('\', '/')
            $inSession = $false
            foreach ($se in ($env:PATH -split ';')) {
                if ([string]::IsNullOrWhiteSpace($se)) { continue }
                if ($se.TrimEnd('\', '/').Equals($normTarget, [System.StringComparison]::OrdinalIgnoreCase)) {
                    $inSession = $true
                    break
                }
            }
            if (-not $inSession) {
                $env:PATH = "$destDir;$env:PATH"
            }
        }

        $installedVer = & $destExe --version
        Write-Host ""
        Write-Host "Installed $installedVer to $destExe"
        Write-Host "Open a new terminal (or run right here) and type: riont"
    } finally {
        Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
    }
} $Version $InstallDir $ArchivePath $NoModifyPath.IsPresent
