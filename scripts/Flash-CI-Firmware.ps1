[CmdletBinding()]
param(
    [string]$Port = '',
    [switch]$ListOnly,
    [switch]$SelfTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Repo = 'waveshareteam/ESP32-C6-Touch-AMOLED-2.16'
$Board = 'ESP32-C6-Touch-AMOLED-2.16'
$DefaultStartIndex = 1
$IdfNames = @('01_AXP2101_Test', '02_I2C_QMI8658', '03_I2C_PCF85063', '04_SD_Card', '05_WIFI_STA', '06_WIFI_AP', '07_Audio_Test', '08_LVGL_V8_Test', '09_LVGL_V9_Test')
$ArduinoNames = @('01_AXP2101_Test', '02_I2C_QMI8658', '03_I2C_PCF85063', '04_SD_Card', '05_WIFI_STA', '06_WIFI_AP', '07_Audio_Test', '08_LVGL_V8_Test', '09_LVGL_V9_Test')
$Items = @()
$index = 1
foreach ($name in $IdfNames) {
    foreach ($version in @('v5.5.5', 'v6.0.2')) {
        $Items += [pscustomobject]@{ Index = $index; Workflow = 'esp-idf-examples.yml'; Artifact = "$name-esp-idf-$version"; Framework = 'esp-idf'; Version = $version; SourceProject = "examples/esp-idf/$name" }
        $index++
    }
}
foreach ($name in $ArduinoNames) {
    $Items += [pscustomobject]@{ Index = $index; Workflow = 'arduino-examples.yml'; Artifact = "$name-arduino-3.3.11"; Framework = 'arduino-esp32'; Version = '3.3.11'; SourceProject = "examples/arduino/$name" }
    $index++
}

function Test-Port([string]$Value) { return $Value -match '^COM\d+$' }
function Get-NextProgress([int]$CurrentIndex, [int[]]$ConfirmedIndexes, [int]$ItemCount) {
    if ($ItemCount -lt 1 -or $CurrentIndex -lt 1 -or $CurrentIndex -gt $ItemCount) { throw 'Progress indexes must be within the item range.' }
    $confirmed = @($ConfirmedIndexes + $CurrentIndex | Where-Object { $_ -ge 1 -and $_ -le $ItemCount } | Sort-Object -Unique)
    return [pscustomobject]@{ CurrentIndex = if ($CurrentIndex -eq $ItemCount) { $CurrentIndex } else { $CurrentIndex + 1 }; ConfirmedIndexes = $confirmed; Completed = $CurrentIndex -eq $ItemCount }
}
function Get-StateForFinalSha($Saved, [string]$ExpectedSha, [string]$DefaultPort) {
    if (-not $Saved -or -not $Saved.PSObject.Properties['FinalSha'] -or -not $Saved.PSObject.Properties['CurrentIndex'] -or -not $Saved.PSObject.Properties['ConfirmedIndexes'] -or [string]$Saved.FinalSha -ne $ExpectedSha) {
        return [pscustomobject]@{ CurrentIndex = $DefaultStartIndex; ConfirmedIndexes = @(); Port = $DefaultPort }
    }
    $current = [int]$Saved.CurrentIndex
    if ($current -lt 1 -or $current -gt $Items.Count) { throw "Saved CurrentIndex is outside 1..$($Items.Count)." }
    return [pscustomobject]@{ CurrentIndex = $current; ConfirmedIndexes = @($Saved.ConfirmedIndexes | ForEach-Object { [int]$_ } | Where-Object { $_ -ge 1 -and $_ -le $Items.Count } | Sort-Object -Unique); Port = $DefaultPort }
}
function Test-RelativePackagePath([string]$PackageRoot, [string]$RelativePath) {
    if ([string]::IsNullOrWhiteSpace($RelativePath) -or [System.IO.Path]::IsPathRooted($RelativePath)) { return $false }
    $root = [System.IO.Path]::GetFullPath($PackageRoot).TrimEnd([System.IO.Path]::DirectorySeparatorChar, [System.IO.Path]::AltDirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar
    $candidate = [System.IO.Path]::GetFullPath((Join-Path $PackageRoot $RelativePath))
    return $candidate.StartsWith($root, [System.StringComparison]::OrdinalIgnoreCase)
}
function Get-FileSha256([string]$Path) {
    $stream = $null; $algorithm = $null
    try { $stream = [System.IO.File]::OpenRead($Path); $algorithm = [System.Security.Cryptography.SHA256]::Create(); return [System.BitConverter]::ToString($algorithm.ComputeHash($stream)).Replace('-', '').ToLowerInvariant() }
    finally { if ($null -ne $stream) { $stream.Dispose() }; if ($null -ne $algorithm) { $algorithm.Dispose() } }
}
if ($SelfTest) {
    $current = $DefaultStartIndex; $confirmed = @(); $transitions = 0
    while ($current -lt $Items.Count) { $next = Get-NextProgress $current $confirmed $Items.Count; if ($next.Completed -or $next.CurrentIndex -ne ($current + 1)) { throw 'SelfTest expected one-item progress.' }; $current = $next.CurrentIndex; $confirmed = @($next.ConfirmedIndexes); $transitions++ }
    $last = Get-NextProgress $current $confirmed $Items.Count
    if (-not $last.Completed -or @($last.ConfirmedIndexes).Count -ne $Items.Count) { throw 'SelfTest did not complete every item.' }
    $reset = Get-StateForFinalSha ([pscustomobject]@{ FinalSha = 'different'; CurrentIndex = 4; ConfirmedIndexes = @(1,2,3) }) 'expected' ''
    if ($reset.CurrentIndex -ne 1 -or @($reset.ConfirmedIndexes).Count -ne 0) { throw 'SelfTest did not reset state for a new SHA.' }
    $packageRoot = Join-Path ([System.IO.Path]::GetTempPath()) 'package'
    $parentEscape = Join-Path '..' 'escape.bin'
    $absoluteEscape = [System.IO.Path]::GetFullPath((Join-Path ([System.IO.Path]::GetTempPath()) 'escape.bin'))
    $insidePath = Join-Path 'bin' 'app.bin'
    if ((Test-RelativePackagePath $packageRoot $parentEscape) -or (Test-RelativePackagePath $packageRoot $absoluteEscape) -or -not (Test-RelativePackagePath $packageRoot $insidePath)) { throw 'SelfTest relative manifest path validation failed.' }
    Write-Output 'SELF_TEST_OK startIndex=1 transitions=26 completed=27'
    return
}
if ($ListOnly) {
    Write-Output 'finalSHA=resolved-at-runtime'; Write-Output 'defaultPort=auto-detect-at-runtime'; Write-Output "startIndex=$DefaultStartIndex"
    foreach ($item in $Items) { Write-Output ('{0}: workflow={1} run=resolved-at-runtime artifact={2} source={3}' -f $item.Index, $item.Workflow, $item.Artifact, $item.SourceProject) }
    return
}
function Resolve-DefaultPort {
    $ports = @(Get-CimInstance Win32_PnPEntity -ErrorAction SilentlyContinue | Where-Object { $_.PNPDeviceID -match 'VID_303A&PID_1001' -and $_.Name -match '\(COM\d+\)' } | ForEach-Object { [regex]::Match($_.Name, '\((COM\d+)\)').Groups[1].Value } | Sort-Object -Unique)
    if ($ports.Count -eq 1) { return $ports[0] }
    throw 'Unable to identify exactly one ESP32-C6 USB serial port; pass -Port COMx.'
}
$RepoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$StateRoot = Join-Path $env:LOCALAPPDATA 'Waveshare\ESP32-C6-Touch-AMOLED-2.16\ci-firmware'
$StatePath = Join-Path $StateRoot 'state-v1.json'
function Resolve-Executable([string]$Name, [string[]]$Fallbacks) {
    $command = Get-Command $Name -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($command -and $command.Source) { return $command.Source }
    foreach ($candidate in $Fallbacks) { if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate } }
    throw "$Name was not found on PATH or in supported fallback locations."
}
function Resolve-Git { return Resolve-Executable 'git' @((Join-Path ${env:ProgramFiles} 'Git\cmd\git.exe'), (Join-Path ${env:ProgramFiles} 'Git\bin\git.exe'), 'C:\Git\cmd\git.exe', 'D:\Git\cmd\git.exe') }
function Resolve-Gh { return Resolve-Executable 'gh' @((Join-Path ${env:ProgramFiles} 'GitHub CLI\gh.exe'), (Join-Path ${env:ProgramFiles} 'GitHub CLI\bin\gh.exe')) }
function Resolve-PythonWithEsptool {
    $command = Get-Command python -ErrorAction SilentlyContinue | Select-Object -First 1; $candidates = @(); if ($command -and $command.Source) { $candidates += $command.Source }
    foreach ($root in @((Join-Path $env:USERPROFILE '.espressif\python_env'), 'C:\Espressif', 'D:\espressif')) { if (Test-Path -LiteralPath $root) { $candidates += @(Get-ChildItem -LiteralPath $root -Recurse -File -Filter python.exe -ErrorAction SilentlyContinue | Where-Object { $_.FullName -match '[\\/]python_env[\\/].+[\\/]Scripts[\\/]python\.exe$' } | ForEach-Object FullName) } }
    foreach ($candidate in @($candidates | Select-Object -Unique)) { & $candidate -c 'import esptool' *> $null; if ($LASTEXITCODE -eq 0) { return $candidate } }
    throw 'No Python interpreter with esptool was found.'
}
function Resolve-FinalSha([string]$GitExe) { $sha = (& $GitExe -C $RepoRoot rev-parse HEAD 2>&1 | Out-String).Trim(); if ($LASTEXITCODE -ne 0 -or $sha -notmatch '^[0-9a-fA-F]{40}$') { throw 'Unable to resolve a full local git HEAD SHA.' }; return $sha.ToLowerInvariant() }
function Assert-CleanWorktree([string]$GitExe) { $status = (& $GitExe -C $RepoRoot status --porcelain=v1 --untracked-files=all 2>&1 | Out-String); if ($LASTEXITCODE -ne 0 -or -not [string]::IsNullOrWhiteSpace($status)) { throw 'Refusing to continue: the working tree has staged, unstaged, or untracked changes.' } }
function Resolve-CurrentBranch([string]$GitExe) { $branch = (& $GitExe -C $RepoRoot symbolic-ref --quiet --short HEAD 2>&1 | Out-String).Trim(); if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($branch)) { throw 'Refusing to continue: check out a non-detached branch first.' }; return $branch }
function Assert-ReadyPullRequest([string]$GhExe, [string]$Branch, [string]$FinalSha) {
    $raw = (& $GhExe pr list --repo $Repo --head $Branch --state open --limit 2 --json number,state,isDraft,headRefName,headRefOid 2>&1 | Out-String); if ($LASTEXITCODE -ne 0) { throw 'Unable to query the open pull request for the current branch.' }
    $pullRequests = @($raw | ConvertFrom-Json); if ($pullRequests.Count -ne 1) { throw 'Refusing to continue: the current branch must have exactly one open pull request.' }; $pullRequest = $pullRequests[0]
    if ([string]$pullRequest.state -ine 'OPEN' -or [bool]$pullRequest.isDraft -or [string]$pullRequest.headRefName -ne $Branch -or -not [string]::Equals([string]$pullRequest.headRefOid, $FinalSha, [System.StringComparison]::OrdinalIgnoreCase)) { throw 'Refusing to continue: the pull request must be non-draft, match the current branch, and have the complete local HEAD SHA.' }
}
function Resolve-ArtifactRuns([string]$GhExe, [string]$FinalSha) {
    $runByWorkflow = @{}; foreach ($workflow in @($Items.Workflow | Sort-Object -Unique)) { $raw = (& $GhExe run list --repo $Repo --workflow $workflow --commit $FinalSha --status success --limit 20 --json databaseId,headSha,createdAt 2>&1 | Out-String); if ($LASTEXITCODE -ne 0) { throw "Unable to list successful $workflow runs: $raw" }; $runs = @($raw | ConvertFrom-Json | Where-Object { $_.headSha -eq $FinalSha } | Sort-Object createdAt -Descending); if ($runs.Count -lt 1) { throw "No successful $workflow run exists for local HEAD $FinalSha." }; $runByWorkflow[$workflow] = [string]$runs[0].databaseId }; foreach ($item in $Items) { $item | Add-Member -NotePropertyName Run -NotePropertyValue $runByWorkflow[$item.Workflow] -Force }
}
function Ensure-StateRoot { if (-not (Test-Path -LiteralPath $StateRoot)) { New-Item -ItemType Directory -Path $StateRoot | Out-Null } }
function Read-State([string]$FinalSha) { $saved = if (Test-Path -LiteralPath $StatePath) { Get-Content -LiteralPath $StatePath -Raw | ConvertFrom-Json } else { $null }; return Get-StateForFinalSha $saved $FinalSha $Port }
function Save-State([int]$CurrentIndex, [int[]]$ConfirmedIndexes, [string]$SavedPort, [string]$FinalSha) { Ensure-StateRoot; [pscustomobject]@{ CurrentIndex = $CurrentIndex; ConfirmedIndexes = @($ConfirmedIndexes | Sort-Object -Unique); Port = $SavedPort; UpdatedAt = (Get-Date).ToString('o'); Repository = $Repo; FinalSha = $FinalSha } | ConvertTo-Json | Set-Content -LiteralPath $StatePath -Encoding UTF8 }
function New-RunPaths { Ensure-StateRoot; $stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'; $downloadRoot = Join-Path $StateRoot 'downloads'; $logRoot = Join-Path $StateRoot 'logs'; foreach ($dir in @($downloadRoot, $logRoot)) { if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null } }; $downloadDir = Join-Path $downloadRoot $stamp; $logPath = Join-Path $logRoot ($stamp + '.log'); if ((Test-Path -LiteralPath $downloadDir) -or (Test-Path -LiteralPath $logPath)) { throw "Timestamp collision at $stamp; no existing files were changed." }; New-Item -ItemType Directory -Path $downloadDir | Out-Null; New-Item -ItemType File -Path $logPath | Out-Null; return [pscustomobject]@{ DownloadDir = $downloadDir; LogPath = $logPath } }
function Add-RunLog([string]$Path, [string]$Text) { Add-Content -LiteralPath $Path -Value $Text -Encoding UTF8 }
function Expand-SafeZip([string]$ZipPath, [string]$Destination) { Add-Type -AssemblyName System.IO.Compression.FileSystem; $archive = [System.IO.Compression.ZipFile]::OpenRead($ZipPath); try { foreach ($entry in $archive.Entries) { if (-not (Test-RelativePackagePath $Destination $entry.FullName)) { throw "Unsafe ZIP entry: $($entry.FullName)" } } } finally { $archive.Dispose() }; Expand-Archive -LiteralPath $ZipPath -DestinationPath $Destination -ErrorAction Stop }
function Find-PackageDirectory([string]$DownloadDir) { $zips = @(Get-ChildItem -LiteralPath $DownloadDir -Recurse -File -Filter '*.zip'); foreach ($zip in $zips) { $destination = Join-Path $zip.DirectoryName ($zip.BaseName + '-unzipped'); if (Test-Path -LiteralPath $destination) { throw "Refusing to overwrite extraction directory: $destination" }; Expand-SafeZip $zip.FullName $destination }; $manifests = @(Get-ChildItem -LiteralPath $DownloadDir -Recurse -File -Filter 'manifest.json'); if ($manifests.Count -ne 1) { throw 'Expected exactly one manifest.json in the downloaded artifact.' }; return $manifests[0].DirectoryName }
function Test-PackageManifest([string]$PackageDir, $Item, [string]$FinalSha) {
    $manifestPath = Join-Path $PackageDir 'manifest.json'; if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { throw 'Package manifest.json is missing.' }; $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ($manifest.schema_version -ne 1 -or $manifest.board -ne $Board -or $manifest.chip -ne 'esp32c6' -or $manifest.git_sha -ne $FinalSha -or $manifest.framework -ne $Item.Framework -or $manifest.framework_version -ne $Item.Version -or $manifest.source_project -ne $Item.SourceProject -or $manifest.flash.baud -ne 921600 -or @($manifest.files).Count -lt 1) { throw 'Package manifest identity does not match the selected item and local HEAD.' }
    $plan = @(); $offsets = @{}; foreach ($file in @($manifest.files)) { $relativePath = [string]$file.archive_path; if (-not (Test-RelativePackagePath $PackageDir $relativePath) -or [string]$file.sha256 -notmatch '^[0-9a-fA-F]{64}$' -or [int64]$file.size -le 0 -or [string]$file.offset -notmatch '^0x[0-9a-fA-F]+$') { throw "Manifest file metadata is unsafe: $relativePath" }; $fullPath = Join-Path $PackageDir $relativePath; if (-not (Test-Path -LiteralPath $fullPath -PathType Leaf) -or (Get-FileSha256 $fullPath) -ne [string]$file.sha256 -or [int64](Get-Item -LiteralPath $fullPath).Length -ne [int64]$file.size) { throw "Manifest checksum or size verification failed: $relativePath" }; $offset = [Convert]::ToInt64(([string]$file.offset).Substring(2), 16); if ($offsets.ContainsKey($offset) -or $offset + [int64]$file.size -gt 16MB) { throw "Manifest flash range is unsafe: $relativePath" }; $offsets[$offset] = $true; $plan += [pscustomobject]@{ Offset = $offset; Size = [int64]$file.size; Path = $fullPath } }
    $orderedPlan = @($plan | Sort-Object Offset); for ($i = 1; $i -lt $orderedPlan.Count; ++$i) { if ($orderedPlan[$i - 1].Offset + $orderedPlan[$i - 1].Size -gt $orderedPlan[$i].Offset) { throw 'Package manifest contains overlapping flash ranges.' } }; return $orderedPlan
}
function Invoke-CurrentFlash($Item, [string]$SelectedPort, [string]$GhExe, [string]$PythonExe, [string]$FinalSha) { $paths = New-RunPaths; Add-RunLog $paths.LogPath "finalSHA=$FinalSha index=$($Item.Index) artifact=$($Item.Artifact) run=$($Item.Run) port=$SelectedPort"; $downloadOutput = (& $GhExe run download $Item.Run --repo $Repo --name $Item.Artifact --dir $paths.DownloadDir 2>&1 | Out-String); $downloadExit = $LASTEXITCODE; Add-RunLog $paths.LogPath $downloadOutput; if ($downloadExit -ne 0) { throw "Artifact download failed with exit code $downloadExit. Log: $($paths.LogPath)" }; $plan = Test-PackageManifest (Find-PackageDirectory $paths.DownloadDir) $Item $FinalSha; $args = @('-m', 'esptool', '--port', $SelectedPort, '--chip', 'esp32c6', '--baud', '921600', 'write_flash'); foreach ($entry in $plan) { $args += ('0x{0:X}' -f $entry.Offset); $args += $entry.Path }; $flashOutput = (& $PythonExe @args 2>&1 | Out-String); $flashExit = $LASTEXITCODE; Add-RunLog $paths.LogPath $flashOutput; $verified = ($flashExit -eq 0) -and $flashOutput.Contains('Hash of data verified'); return [pscustomobject]@{ Success = $verified; Output = $flashOutput; LogPath = $paths.LogPath; Detail = if ($verified) { 'Flash completed and Hash of data verified was found.' } else { 'Flash did not meet the required exit-code and hash-verification condition.' } } }

$GitExe = Resolve-Git; $FinalSha = Resolve-FinalSha $GitExe; Assert-CleanWorktree $GitExe; $Branch = Resolve-CurrentBranch $GitExe; $GhExe = Resolve-Gh; Assert-ReadyPullRequest $GhExe $Branch $FinalSha; $PythonExe = Resolve-PythonWithEsptool
if ([string]::IsNullOrWhiteSpace($Port)) { $Port = Resolve-DefaultPort }; $Port = $Port.Trim().ToUpperInvariant(); if (-not (Test-Port $Port)) { throw 'Port must be COM followed by digits, for example COMx.' }; Resolve-ArtifactRuns $GhExe $FinalSha
Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing
$state = Read-State $FinalSha; $script:CurrentIndex = $state.CurrentIndex; $script:ConfirmedIndexes = @($state.ConfirmedIndexes); $script:CurrentFlashVerified = $false
$form = New-Object System.Windows.Forms.Form; $form.Text = 'CI Firmware Flasher'; $form.StartPosition = 'CenterScreen'; $form.ClientSize = New-Object System.Drawing.Size(820, 670); $form.FormBorderStyle = 'FixedDialog'; $form.MaximizeBox = $false
function Add-Label([string]$Text, [int]$X, [int]$Y, [int]$Width = 780) { $label = New-Object System.Windows.Forms.Label; $label.Text = $Text; $label.Location = New-Object System.Drawing.Point($X, $Y); $label.Size = New-Object System.Drawing.Size($Width, 20); $form.Controls.Add($label); return $label }
$repoLabel = Add-Label "Repository: $Repo" 15 15; $shaLabel = Add-Label "Final SHA: $FinalSha" 15 40; $portCaption = Add-Label 'Port:' 15 70 45; $portBox = New-Object System.Windows.Forms.TextBox; $portBox.Text = $state.Port; $portBox.Location = New-Object System.Drawing.Point(65, 67); $portBox.Size = New-Object System.Drawing.Size(110, 22); $form.Controls.Add($portBox); $currentLabel = Add-Label '' 15 100; $statusLabel = Add-Label 'Status: Select Flash current to begin.' 15 125
$progressList = New-Object System.Windows.Forms.ListBox; $progressList.Font = New-Object System.Drawing.Font('Consolas', 9); $progressList.Location = New-Object System.Drawing.Point(15, 155); $progressList.Size = New-Object System.Drawing.Size(790, 250); $form.Controls.Add($progressList); $outputBox = New-Object System.Windows.Forms.TextBox; $outputBox.Multiline = $true; $outputBox.ReadOnly = $true; $outputBox.ScrollBars = 'Both'; $outputBox.WordWrap = $false; $outputBox.Font = New-Object System.Drawing.Font('Consolas', 9); $outputBox.Location = New-Object System.Drawing.Point(15, 415); $outputBox.Size = New-Object System.Drawing.Size(790, 190); $form.Controls.Add($outputBox)
$flashButton = New-Object System.Windows.Forms.Button; $flashButton.Text = 'Flash current'; $flashButton.Location = New-Object System.Drawing.Point(15, 620); $flashButton.Size = New-Object System.Drawing.Size(145, 32); $form.Controls.Add($flashButton); $confirmButton = New-Object System.Windows.Forms.Button; $confirmButton.Text = 'Mark PASS and flash next'; $confirmButton.Location = New-Object System.Drawing.Point(170, 620); $confirmButton.Size = New-Object System.Drawing.Size(215, 32); $confirmButton.Enabled = $false; $form.Controls.Add($confirmButton); $exitButton = New-Object System.Windows.Forms.Button; $exitButton.Text = 'Exit'; $exitButton.Location = New-Object System.Drawing.Point(685, 620); $exitButton.Size = New-Object System.Drawing.Size(120, 32); $form.Controls.Add($exitButton)
function Update-CurrentDisplay { $item = $Items[$script:CurrentIndex - 1]; $currentLabel.Text = "Current: $($item.Index)/$($Items.Count) Artifact: $($item.Artifact) Run: $($item.Run)"; $progressList.Items.Clear(); foreach ($progressItem in $Items) { $prefix = if ($script:ConfirmedIndexes -contains $progressItem.Index) { '[PASS]' } elseif ($progressItem.Index -eq $script:CurrentIndex) { '[CURRENT]' } else { '[WAIT]' }; [void]$progressList.Items.Add(('{0} {1}: {2}' -f $prefix, $progressItem.Index, $progressItem.Artifact)) }; $progressList.SelectedIndex = $script:CurrentIndex - 1 }
function Set-Busy([bool]$Busy) { $complete = $script:CurrentIndex -eq $Items.Count -and $script:ConfirmedIndexes -contains $Items.Count; $flashButton.Enabled = (-not $Busy) -and (-not $complete); $confirmButton.Enabled = (-not $Busy) -and $script:CurrentFlashVerified -and (-not $complete); $exitButton.Enabled = -not $Busy; $portBox.Enabled = -not $Busy; $form.UseWaitCursor = $Busy; [System.Windows.Forms.Application]::DoEvents() }
function Flash-CurrentItem { $selectedPort = $portBox.Text.Trim().ToUpperInvariant(); if (-not (Test-Port $selectedPort)) { [System.Windows.Forms.MessageBox]::Show('Port must be COM followed by digits, for example COMx.', 'Invalid port') | Out-Null; return }; $script:CurrentFlashVerified = $false; Set-Busy $true; $item = $Items[$script:CurrentIndex - 1]; $statusLabel.Text = "Status: Flashing item $($item.Index) on $selectedPort..."; try { $result = Invoke-CurrentFlash $item $selectedPort $GhExe $PythonExe $FinalSha; $outputBox.Text = "Log: $($result.LogPath)`r`n`r`n$($result.Output)"; if ($result.Success) { Save-State $script:CurrentIndex $script:ConfirmedIndexes $selectedPort $FinalSha; $statusLabel.Text = "Status: $($result.Detail) Confirm after checking the device."; $script:CurrentFlashVerified = $true } else { $statusLabel.Text = "Status: $($result.Detail) Current item was not advanced. Log: $($result.LogPath)" } } catch { $outputBox.Text = $_ | Out-String; $statusLabel.Text = "Status: Error. Current item was not advanced. $($_.Exception.Message)" } finally { Set-Busy $false } }
$flashButton.Add_Click({ Flash-CurrentItem }); $confirmButton.Add_Click({ if (-not $script:CurrentFlashVerified) { return }; $selectedPort = $portBox.Text.Trim().ToUpperInvariant(); $next = Get-NextProgress $script:CurrentIndex $script:ConfirmedIndexes $Items.Count; $script:CurrentIndex = $next.CurrentIndex; $script:ConfirmedIndexes = @($next.ConfirmedIndexes); $script:CurrentFlashVerified = $false; Save-State $script:CurrentIndex $script:ConfirmedIndexes $selectedPort $FinalSha; Update-CurrentDisplay; if ($next.Completed) { Set-Busy $false; $statusLabel.Text = "Status: All $($Items.Count) items are confirmed."; return }; Flash-CurrentItem }); $exitButton.Add_Click({ $form.Close() }); $progressList.Add_SelectedIndexChanged({ if ($progressList.SelectedIndex -ne ($script:CurrentIndex - 1)) { $progressList.SelectedIndex = $script:CurrentIndex - 1 } }); Update-CurrentDisplay; if ($script:CurrentIndex -eq $Items.Count -and $script:ConfirmedIndexes -contains $Items.Count) { Set-Busy $false; $statusLabel.Text = "Status: All $($Items.Count) items are confirmed." }; [void]$form.ShowDialog()
