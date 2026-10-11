# kterm shell glue: PowerShell, cmd and Linux commands in one prompt.
# kterm rewrites this file on every launch; put personal tweaks in your normal $PROFILE.
# ASCII only: Windows PowerShell 5.1 reads BOM-less scripts in the ANSI codepage.

$e = [char]27
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$OutputEncoding = [Text.Encoding]::UTF8

# ---- Linux commands: Git for Windows ships real coreutils (grep, sed, awk, less, ...) --------
$kGitUsr = @("$env:ProgramFiles\Git\usr\bin", "$env:LOCALAPPDATA\Programs\Git\usr\bin") |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if ($kGitUsr) {
    # Appended, so Windows tools keep priority; the clashes are handled below.
    $env:PATH = "$env:PATH;$kGitUsr"
    # These PowerShell aliases hide the real tools. (sort stays Sort-Object: it's used in pipelines.)
    foreach ($a in 'ls', 'cat', 'rm', 'cp', 'mv', 'diff', 'curl', 'wget') {
        Remove-Item "Alias:$a" -Force -ErrorAction SilentlyContinue
    }
    function global:ls { & "$kGitUsr\ls.exe" --color=auto -F @args }
    function global:ll { & "$kGitUsr\ls.exe" --color=auto -lAhF @args }
    function global:grep { & "$kGitUsr\grep.exe" --color=auto @args }
    # find.exe exists in System32 too and wins on PATH.
    function global:find { & "$kGitUsr\find.exe" @args }
}

# ---- cmd syntax: lines that only make sense to cmd.exe are handed to it --------------------
$kCmdPattern = @(
    '%\w+%'                                    # %VAR%
    '^\s*set(\s*$|\s+\w+=)'                    # set / set X=1
    '^\s*(mklink|assoc|ftype|ver|vol)\b'       # cmd-only builtins
    '^\s*(dir|copy|del|erase|move|ren|rd|md|rmdir|mkdir|xcopy|attrib|tree)\b.*\s/[a-z?]'  # /switches
    '^\s*for\s+%'                              # for %i in ...
    '^\s*if\s+(not\s+)?(exist|errorlevel|defined)\b'
    '^\s*echo\.'
) -join '|'

function global:Test-KtermCmdLine([string]$line) {
    if ($line -match $kCmdPattern) { return $true }
    # Windows PowerShell 5.1 has no && / ||; cmd does.
    return $PSVersionTable.PSVersion.Major -lt 7 -and $line -match '&&|\|\|'
}

if (Get-Module -ListAvailable PSReadLine) {
    Import-Module PSReadLine
    Set-PSReadLineKeyHandler -Key Enter -ScriptBlock {
        $line = $null; $cursor = $null
        [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)
        if ($line -and (Test-KtermCmdLine $line)) {
            # --% passes the rest to cmd untouched.
            [Microsoft.PowerShell.PSConsoleReadLine]::Replace(0, $line.Length, "cmd --% /d /c $line")
        }
        [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
    }
    Set-PSReadLineKeyHandler -Key Tab -Function MenuComplete
    Set-PSReadLineKeyHandler -Key UpArrow -Function HistorySearchBackward
    Set-PSReadLineKeyHandler -Key DownArrow -Function HistorySearchForward
    Set-PSReadLineOption -HistoryNoDuplicates
    try { Set-PSReadLineOption -PredictionSource History } catch {}  # PSReadLine 2.1+
}

# ---- kterm settings: `config` (live menu) and `hideme` ------------------------------------------
# Both edit %APPDATA%\kterm\config.json; kterm applies changes within a moment.

$kCfgPath = "$env:APPDATA\kterm\config.json"
$kCfgDefaults = [ordered]@{
    font_family = 'JetBrainsMono Nerd Font'; font_size = 11.0; opacity = 0.92; padding = 12.0
    cursor_shape = 'block'; cursor_blink = $true; cursor_trail = $true; trail_speed = 1.0
    accent = 'auto'; gaps = 10.0; hide_user = $false
}

function global:Get-KtermConfig {
    $c = [ordered]@{}
    foreach ($k in $kCfgDefaults.Keys) { $c[$k] = $kCfgDefaults[$k] }
    if (Test-Path $kCfgPath) {
        try {
            $j = Get-Content $kCfgPath -Raw | ConvertFrom-Json
            foreach ($p in $j.PSObject.Properties) { if ($c.Contains($p.Name)) { $c[$p.Name] = $p.Value } }
        } catch {}
    }
    $c
}

# '#rrggbb' -> 'r;g;b' for 24-bit SGR; 'auto' -> the Windows accent kterm passed in.
function global:ConvertTo-KtermSgr([string]$hex) {
    if ($hex -match '^#([0-9a-fA-F]{2})([0-9a-fA-F]{2})([0-9a-fA-F]{2})$') {
        return '{0};{1};{2}' -f [Convert]::ToInt32($Matches[1], 16), [Convert]::ToInt32($Matches[2], 16), [Convert]::ToInt32($Matches[3], 16)
    }
    return $env:KTERM_ACCENT
}

function global:Save-KtermConfig($c) {
    # WriteAllText writes no BOM; kterm's JSON parser rejects one.
    [IO.File]::WriteAllText($kCfgPath, ($c | ConvertTo-Json))
    $global:kHideUser = [bool]$c.hide_user
    $global:kAccent = ConvertTo-KtermSgr $c.accent
}

$kStartCfg = Get-KtermConfig
$global:kHideUser = [bool]$kStartCfg.hide_user
$global:kAccent = ConvertTo-KtermSgr $kStartCfg.accent

function global:hideme {
    $c = Get-KtermConfig
    $c.hide_user = -not [bool]$c.hide_user
    Save-KtermConfig $c
    if ($c.hide_user) { 'Username hidden. Run hideme again to show it.' } else { 'Username visible again.' }
}

function global:config {
    $c = Get-KtermConfig
    Add-Type -AssemblyName System.Drawing
    # GDI reports Nerd Fonts by their short legacy names ("X NF"); kterm (DirectWrite) uses the full ones.
    $installed = @((New-Object System.Drawing.Text.InstalledFontCollection).Families | ForEach-Object {
        $_.Name -replace ' NF$', ' Nerd Font' -replace ' NFM$', ' Nerd Font Mono' -replace ' NFP$', ' Nerd Font Propo'
    })
    # Monospace-looking families, without the per-weight entries Windows lists separately.
    $fonts = @($installed | Where-Object {
        $_ -match '\b(Mono|Code|Consolas|Hack|Iosevka|Nerd|NF|NFM|Fira|Courier|Lucida Console)\b' -and
        $_ -notmatch '(Extra|Semi|Demi)?(Light|Bold|Thin|Medium|Black)$'
    } | Sort-Object)
    if ($fonts.Count -eq 0) { $fonts = @('Consolas') }
    # kterm's own fallback order, to say what is really drawn when the configured font is missing.
    $fallback = @('JetBrainsMono Nerd Font', 'JetBrains Mono', 'Cascadia Mono', 'Consolas') | Where-Object { $installed -contains $_ } | Select-Object -First 1
    # Keep the configured font selectable even if it isn't installed, so you can always go back.
    if ($fonts -notcontains $c.font_family) { $fonts = @($c.font_family) + $fonts }
    $accents = @('auto', '#1793d1', '#cba6f7', '#f38ba8', '#fab387', '#f9e2af', '#a6e3a1', '#89dceb', '#b4befe')
    $items = @(
        @{ k = 'font_family';  n = 'Font';              t = 'list'; v = $fonts }
        @{ k = 'font_size';    n = 'Font size';         t = 'num';  min = 7;    max = 28; step = 0.5 }
        @{ k = 'opacity';      n = 'Opacity';           t = 'num';  min = 0.3;  max = 1;  step = 0.02 }
        @{ k = 'padding';      n = 'Padding';           t = 'num';  min = 0;    max = 48; step = 2 }
        @{ k = 'cursor_shape'; n = 'Cursor shape';      t = 'list'; v = @('block', 'beam', 'underline') }
        @{ k = 'cursor_blink'; n = 'Cursor blink';      t = 'bool' }
        @{ k = 'cursor_trail'; n = 'Cursor trail';      t = 'bool' }
        @{ k = 'trail_speed';  n = 'Trail speed';       t = 'num';  min = 0.25; max = 3;  step = 0.25 }
        @{ k = 'accent';       n = 'Accent color';      t = 'list'; v = $accents }
        @{ k = 'gaps';         n = 'Tiling gaps';       t = 'num';  min = 0;    max = 40; step = 2 }
        @{ k = 'hide_user';    n = 'Hide username';     t = 'bool' }
        @{ k = 'reset';        n = 'Reset to defaults'; t = 'action' }
    )
    $sel = 0

    [Console]::Write("$e[?1049h$e[?25l")  # alternate screen, hidden cursor
    try {
        while ($true) {
            $acc = $global:kAccent
            $sb = New-Object Text.StringBuilder
            [void]$sb.Append("$e[H$e[2J`r`n  $e[1;38;2;${acc}mkterm config$e[0m`r`n`r`n")
            for ($i = 0; $i -lt $items.Count; $i++) {
                $it = $items[$i]
                $v = $c[$it.k]
                $val = switch ($it.t) {
                    'bool'   { if ($v) { 'on' } else { 'off' } }
                    'num'    { '{0:0.##}' -f [double]$v }
                    'action' { '' }
                    default {
                        if ($it.k -eq 'accent') {
                            $label = if ($v -eq 'auto') { 'auto (Windows)' } else { $v }
                            "$e[48;2;$(ConvertTo-KtermSgr $v)m    $e[0m $label"
                        } elseif ($it.k -eq 'font_family' -and $installed -notcontains $v) {
                            "$v $e[33m(not installed - showing $fallback)$e[0m"
                        } else { "$v" }
                    }
                }
                if ($i -eq $sel) {
                    $shown = if ($it.t -eq 'action') { "$e[2m(enter)$e[0m" } else { "$e[38;2;${acc}m<$e[0m $val $e[38;2;${acc}m>$e[0m" }
                    [void]$sb.Append(("  $e[1;38;2;${acc}m> {0,-18}$e[0m {1}`r`n" -f $it.n, $shown))
                } else {
                    [void]$sb.Append(("    $e[2m{0,-18}$e[0m   {1}`r`n" -f $it.n, $val))
                }
            }
            [void]$sb.Append("`r`n  $e[2mup/down select   left/right change   enter custom color   q quit$e[0m")
            [Console]::Write($sb.ToString())

            # Wait for a key, but redraw if the terminal resizes (e.g. after a font size change);
            # otherwise the console reflows the old menu into a mess.
            $size = "$([Console]::WindowWidth)x$([Console]::WindowHeight)"
            while (-not [Console]::KeyAvailable -and "$([Console]::WindowWidth)x$([Console]::WindowHeight)" -eq $size) {
                Start-Sleep -Milliseconds 40
            }
            if (-not [Console]::KeyAvailable) { continue }
            $key = [Console]::ReadKey($true)
            $it = $items[$sel]
            $dir = 0
            $ch = [string]$key.KeyChar
            if ($key.Key -eq 'Escape' -or $ch -eq 'q') { return }
            if ($key.Key -eq 'UpArrow' -or $ch -eq 'k') { $sel = ($sel - 1 + $items.Count) % $items.Count; continue }
            if ($key.Key -eq 'DownArrow' -or $ch -eq 'j') { $sel = ($sel + 1) % $items.Count; continue }
            if ($key.Key -eq 'LeftArrow' -or $ch -eq 'h') { $dir = -1 }
            if ($key.Key -eq 'RightArrow' -or $ch -eq 'l' -or $key.Key -eq 'Enter') { $dir = 1 }
            if ($dir -eq 0) { continue }

            if ($it.t -eq 'action') {
                if ($key.Key -ne 'Enter') { continue }
                $c = [ordered]@{}
                foreach ($k in $kCfgDefaults.Keys) { $c[$k] = $kCfgDefaults[$k] }
            } elseif ($it.k -eq 'accent' -and $key.Key -eq 'Enter') {
                [Console]::Write("$e[?25h`r`n`r`n")
                $hex = (Read-Host '  Hex color, e.g. #1793d1').Trim()
                [Console]::Write("$e[?25l")
                if ($hex -and $hex -notmatch '^#') { $hex = "#$hex" }
                if ($hex -match '^#[0-9a-fA-F]{6}$') { $c.accent = $hex.ToLower() }
            } else {
                switch ($it.t) {
                    'num' {
                        $x = [double]$c[$it.k] + $dir * $it.step
                        $c[$it.k] = [math]::Round([math]::Min($it.max, [math]::Max($it.min, $x)), 2)
                    }
                    'bool' { $c[$it.k] = -not [bool]$c[$it.k] }
                    'list' {
                        $n = $it.v.Count
                        $i = [array]::IndexOf($it.v, [string]$c[$it.k])
                        $c[$it.k] = $it.v[(($i + $dir) % $n + $n) % $n]
                    }
                }
            }
            Save-KtermConfig $c
        }
    } finally {
        [Console]::Write("$e[?25h$e[?1049l")
    }
}

# ---- Arch-style prompt:  user@host ~/path >  (accent colored, red after a failure) -----------
function global:prompt {
    $ok = $?
    $acc = if ($global:kAccent) { "38;2;$global:kAccent" } else { '34' }
    $user = if ($global:kHideUser) { '*****' } else { $env:USERNAME }
    $p = $PWD.ProviderPath
    if ($p.StartsWith($HOME, [StringComparison]::OrdinalIgnoreCase)) { $p = '~' + $p.Substring($HOME.Length) }
    $p = $p -replace '\\', '/'
    $arrow = if ($ok) { $acc } else { '31' }
    "$e[1;${acc}m$user@$($env:COMPUTERNAME.ToLower())$e[0m $e[1m$p$e[0m $e[${arrow}m$([char]0x276F)$e[0m "
}
