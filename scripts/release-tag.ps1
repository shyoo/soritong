<#
.SYNOPSIS
    Plan or cut an annotated release tag for soritong.

.DESCRIPTION
    Ensures that tags follow semver (X.Y.Z or X.Y.Z-rc.N), that origin is shyoo/soritong,
    that CI on origin/main passed for the commit, and cuts an annotated tag with release notes.

.PARAMETER Action
    'plan' (default) to show what tag would be cut, or 'cut' to create and push the tag.

.PARAMETER Release
    'rc' to create the next release candidate, 'promote' to cut the final release from the latest RC,
    or a specific version string.

.PARAMETER Bump
    'patch' (default), 'minor', or 'major'.

.PARAMETER NotesFile
    Path to file containing markdown release notes. Required for -Action cut.

.PARAMETER DryRun
    Verify all gates without creating or pushing the tag.
#>
[CmdletBinding()]
param(
    [ValidateSet('plan','cut')][string]$Action = 'plan',
    [string]$Release = 'rc',
    [ValidateSet('patch','minor','major')][string]$Bump = 'patch',
    [string]$NotesFile,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path -Parent $PSScriptRoot)
. (Join-Path $PSScriptRoot 'version.ps1')

function Git {
    $result = & git.exe @args
    if ($LASTEXITCODE -ne 0) { throw "git $($args -join ' ') failed (exit $LASTEXITCODE)" }
    return $result
}

function GhJson {
    $result = & gh @args
    if ($LASTEXITCODE -ne 0) { throw "gh $($args -join ' ') failed (exit $LASTEXITCODE)" }
    return ($result | ConvertFrom-Json)
}

# 1. Verify remote origin
$originUrl = Git remote get-url origin
if ($originUrl -notmatch '^https://github\.com/shyoo/soritong(\.git)?$|^git@github\.com:shyoo/soritong(\.git)?$') {
    throw "origin must be shyoo/soritong (found: $originUrl)"
}

# 2. Check for uncommitted changes
if (Git status --porcelain) {
    throw 'Uncommitted changes present: commit or stash them before releasing'
}

# 3. Fetch tags and main from origin
Git fetch origin main --tags | Out-Null

$tagList = @(Git tag --list 'v*')
$baseline = Get-SoritongVersion
$plan = Get-ReleasePlan -TagNames $tagList -Baseline $baseline -Release $Release -Bump $Bump

$version = $plan.Version
$candidateTag = $plan.Candidate
$commit = if ($candidateTag) { Git rev-parse "$candidateTag^{}" } else { Git rev-parse origin/main }
$tag = "v$version"

# 4. Verify commit ancestry
Git merge-base --is-ancestor $commit origin/main | Out-Null

if ($Action -eq 'plan') {
    [pscustomobject]@{
        Version    = $version
        Tag        = $tag
        Commit     = $commit
        Candidate  = $candidateTag
        Since      = $plan.Since
        Prerelease = ($version -like '*-*')
    } | Format-List
    exit 0
}

# 5. Cutting tag: verify CI on origin/main passed
$main = Git rev-parse --verify main
Git merge-base --is-ancestor $main origin/main | Out-Null

$runs = GhJson api "repos/shyoo/soritong/actions/workflows/ci.yml/runs?head_sha=$commit&event=push&per_page=100"
if (-not @($runs.workflow_runs | Where-Object { $_.head_branch -eq 'main' -and $_.conclusion -eq 'success' }).Count) {
    throw "CI on origin/main must pass for commit $commit before cutting a release"
}

if ($candidateTag) {
    $candidate = GhJson release view $candidateTag --repo shyoo/soritong --json 'isPrerelease,isDraft,assets'
    if (-not $candidate.isPrerelease -or $candidate.isDraft) {
        throw "Candidate tag $candidateTag must be a published prerelease"
    }
    $expectedSetup = "soritong-$($candidateTag.Substring(1))-windows-x64-setup.exe"
    if ($expectedSetup -cnotin $candidate.assets.name -or 'SHA256SUMS.txt' -cnotin $candidate.assets.name) {
        throw "Candidate installer and checksums must be published on GitHub before promotion"
    }
}

if (-not $NotesFile -or -not (Test-Path -LiteralPath $NotesFile)) {
    throw 'Pass -NotesFile with the user-facing release notes'
}
$notes = (Get-Content -LiteralPath $NotesFile -Raw).Trim()
if (-not $notes) {
    throw 'Release notes file cannot be empty'
}

Write-Host "$tag -> $commit (candidate: $candidateTag)" -ForegroundColor Cyan
Write-Host $notes

if ($DryRun) {
    Write-Host 'Dry run: all release tag gates passed; no tag created or pushed.' -ForegroundColor Green
    exit 0
}

Git tag -a $tag $commit -F $NotesFile | Out-Null
Git push origin "refs/tags/$tag" | Out-Null
Write-Host "Pushed tag $tag to https://github.com/shyoo/soritong/releases/tag/$tag" -ForegroundColor Green
