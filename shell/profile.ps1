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
    accent = 'auto'; tiling = $true; gaps = 10.0; hide_user = $false
    key_toggle = 'win+enter'; key_new = 'win+shift+enter'; key_close = 'ctrl+w'; key_hide = 'ctrl+shift+h'; key_copy = 'ctrl+shift+c'; key_paste = 'ctrl+shift+v'
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
    $global:kAccent = ConvertTo-KtermSgr $c.accent
}

$kStartCfg = Get-KtermConfig
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
        @{ k = 'tiling';       n = 'Auto docking';      t = 'bool' }
        @{ k = 'gaps';         n = 'Tiling gaps';       t = 'num';  min = 0;    max = 40; step = 2 }
        @{ k = 'hide_user';    n = 'Hide username';     t = 'bool' }
        @{ k = 'key_toggle';   n = 'Key: open/hide all'; t = 'key' }
        @{ k = 'key_new';      n = 'Key: new window';   t = 'key' }
        @{ k = 'key_close';    n = 'Key: close';        t = 'key' }
        @{ k = 'key_hide';     n = 'Key: hide';         t = 'key' }
        @{ k = 'key_copy';     n = 'Key: copy';         t = 'key' }
        @{ k = 'key_paste';    n = 'Key: paste';        t = 'key' }
        @{ k = 'reset';        n = 'Reset to defaults'; t = 'action' }
    )
    $sel = 0

    [Console]::Write("$e[?1049h$e[?25l")  # alternate screen, hidden cursor
    try {
        while ($true) {
            $acc = $global:kAccent
            $sb = New-Object Text.StringBuilder
            [void]$sb.Append("$e[H$e[2J`r`n  $e[1;38;2;${acc}mKTerm config$e[0m`r`n`r`n")
            for ($i = 0; $i -lt $items.Count; $i++) {
                $it = $items[$i]
                $v = $c[$it.k]
                $val = switch ($it.t) {
                    'bool'   { if ($v) { 'on' } else { 'off' } }
                    'num'    { '{0:0.##}' -f [double]$v }
                    'action' { '' }
                    'key'    { $v }
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
            [void]$sb.Append("`r`n  $e[2mup/down select   left/right change   enter custom color / key   q quit$e[0m")
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
            } elseif ($it.t -eq 'key') {
                if ($key.Key -ne 'Enter') { continue }
                # Typed, not pressed: kterm's own binds would swallow the keystroke before the shell sees it.
                [Console]::Write("$e[?25h`r`n`r`n")
                $combo = (Read-Host '  Key, e.g. ctrl+shift+h; ctrl/shift/alt/win + (letters, digits, f1-f12, escape, enter, tab, space)').Trim().ToLower()
                [Console]::Write("$e[?25l")
                if ($combo -match '^((ctrl|shift|alt|win)\+)*([a-z0-9]|f\d{1,2}|esc|escape|enter|tab|space|backspace)$') { $c[$it.k] = $combo }
            } else {
                switch ($it.t) {
                    'num' {
                        $x = [double]$c[$it.k] + $dir * $it.step
                        # [double] casts: with int min/max PowerShell picks the Int32 overloads and rounds 12.5 to 12.
                        $c[$it.k] = [math]::Round([math]::Min([double]$it.max, [math]::Max([double]$it.min, $x)), 2)
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

# ---- `keybinds`: browse Windows hotkeys and switch them on/off (registry-backed) -----------------
#   keybinds [all|winshortcuts|accessibility|input]   arrows move, left/right/enter toggles, r restarts Explorer, q quits
#   keybinds disable|enable <letter|tab|space|...|winkeys|sticky|filter|toggle|langswitch>   one-shot, no menu
# Windows keeps no registry list of its shortcuts, so the Win+letter names below are a built-in table.
# Win+letter changes are only read when Explorer starts; Win+Shift+<letter> follows the letter.
$kbKeys = @{ A = 'Quick settings'; B = 'Focus tray corner'; C = 'Copilot'; D = 'Show desktop'; E = 'File Explorer'; F = 'Feedback Hub'
    G = 'Game Bar'; H = 'Voice typing'; I = 'Settings'; J = 'Tips / focus'; K = 'Cast'; L = 'Lock screen'; M = 'Minimize all'
    N = 'Notifications'; O = 'Lock orientation'; P = 'Project / display'; Q = 'Search'; R = 'Run'; S = 'Search'
    T = 'Cycle taskbar'; U = 'Accessibility settings'; V = 'Clipboard history'; W = 'Widgets'; X = 'Power user menu'
    Y = 'Windows Mixed Reality'; Z = 'Snap layouts' }
# Non-letter Win+ keys: name -> (virtual-key code, what it does). DisabledHotkeys holds one char per VK code.
$kbSpecial = [ordered]@{ Tab = @(0x09, 'Task view'); Space = @(0x20, 'Switch input language'); Comma = @(0xBC, 'Peek at desktop')
    Period = @(0xBE, 'Emoji panel'); Plus = @(0xBB, 'Magnifier'); Home = @(0x24, 'Minimize other windows'); Pause = @(0x13, 'System info')
    PrtScn = @(0x2C, 'Screenshot to file'); Up = @(0x26, 'Maximize'); Down = @(0x28, 'Restore / minimize'); Left = @(0x25, 'Snap left'); Right = @(0x27, 'Snap right') }
$kbReg = @{
    adv = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced'
    pol = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Policies\Explorer'
    acc = 'HKCU:\Control Panel\Accessibility'
    tog = 'HKCU:\Keyboard Layout\Toggle'
}
$kbFlags = @{ sticky = @('StickyKeys', 'Sticky Keys popup', 'press Shift 5 times'); filter = @('Keyboard Response', 'Filter Keys popup', 'hold right Shift 8s'); toggle = @('ToggleKeys', 'Toggle Keys', 'hold NumLock 5s') }

# The DisabledHotkeys character for a Win+key id, or $null for the other toggles.
function global:Get-KbChar([string]$id) {
    if ($id -match '^[A-Z0-9]$') { return $id.ToUpper() }
    if ($kbSpecial.Contains($id)) { return [string][char]$kbSpecial[$id][0] }
}

function global:Get-KbValue($path, $name) { (Get-ItemProperty $path -Name $name -ErrorAction SilentlyContinue).$name }
function global:Set-KbValue($path, $name, $value, $type) {
    if (-not (Test-Path $path)) { New-Item $path -Force | Out-Null }
    Set-ItemProperty $path $name $value -Type $type
}

# Is the shortcut currently working?
function global:Get-KbState([string]$id) {
    if ($c = Get-KbChar $id) { return -not "$(Get-KbValue $kbReg.adv 'DisabledHotkeys')".Contains($c) }
    switch ($id) {
        'winkeys'    { return -not [bool](Get-KbValue $kbReg.pol 'NoWinKeys') }
        'langswitch' { return @('Hotkey', 'Language Hotkey', 'Layout Hotkey' | ForEach-Object { Get-KbValue $kbReg.tog $_ } | Where-Object { $_ -eq '3' }).Count -eq 0 }
        default      { $f = Get-KbValue "$($kbReg.acc)\$($kbFlags[$id][0])" 'Flags'; return ($null -ne $f) -and [bool]([int]$f -band 4) }  # 4 = HOTKEYACTIVE
    }
}

function global:Set-KbState([string]$id, [bool]$on) {
    if ($c = Get-KbChar $id) {
        $set = @([char[]]"$(Get-KbValue $kbReg.adv 'DisabledHotkeys')" | Where-Object { [string]$_ -cne $c })
        if (-not $on) { $set += [char]$c }
        Set-KbValue $kbReg.adv DisabledHotkeys (-join $set) String
        return
    }
    switch ($id) {
        'winkeys'    { Set-KbValue $kbReg.pol NoWinKeys ([int](-not $on)) DWord }
        'langswitch' { foreach ($n in 'Hotkey', 'Language Hotkey', 'Layout Hotkey') { Set-KbValue $kbReg.tog $n $(if ($on) { '1' } else { '3' }) String } }  # 3 = none, 1 = Alt+Shift
        default {
            $k = "$($kbReg.acc)\$($kbFlags[$id][0])"
            $f = [int](Get-KbValue $k 'Flags')
            Set-KbValue $k Flags ([string]$(if ($on) { $f -bor 4 } else { $f -band -bnot 4 })) String
        }
    }
}

function global:keybinds {
    $verb = "$($args[0])".ToLower(); $what = "$($args[1])".ToUpper()
    if ($verb -in 'disable', 'enable') {
        $id = if ($what -match '^[A-Z0-9]$') { $what } else { $what.ToLower() }
        if (-not (Get-KbChar $id) -and $id -notin 'winkeys', 'langswitch', 'sticky', 'filter', 'toggle') {
            "usage: keybinds disable|enable <letter|$($kbSpecial.Keys -join '|')|winkeys|sticky|filter|toggle|langswitch>"; return
        }
        Set-KbState $id ($verb -eq 'enable')
        "$id ${verb}d. $(if (Get-KbChar $id) { 'Open keybinds and press r (or sign out) to apply.' })"
        return
    }

    $groups = [ordered]@{
        winshortcuts  = @(@{ id = 'winkeys'; name = 'All Win shortcuts'; desc = 'every Win+key (sign out to apply)' }) +
                        @([char[]]'ABCDEFGHIJKLMNOPQRSTUVWXYZ' | Where-Object { $kbKeys["$_"] } | ForEach-Object { @{ id = "$_"; name = "Win+$_"; desc = $kbKeys["$_"] } }) +
                        @($kbSpecial.Keys | ForEach-Object { @{ id = $_; name = "Win+$_"; desc = $kbSpecial[$_][1] } })
        accessibility = @($kbFlags.Keys | Sort-Object | ForEach-Object { @{ id = $_; name = $kbFlags[$_][1]; desc = $kbFlags[$_][2] } })
        input         = @(@{ id = 'langswitch'; name = 'Language switch'; desc = 'Alt+Shift changes keyboard language' })
    }
    $view = if ($verb -eq 'display') { "$($args[1])".ToLower() } else { $verb }
    if ($view -notin $groups.Keys) { $view = 'all' }
    $items = @(if ($view -eq 'all') { $groups.Values | ForEach-Object { $_ } } else { $groups[$view] })
    $sel = 0; $top = 0; $note = ''

    [Console]::Write("$e[?1049h$e[?25l")
    try {
        while ($true) {
            $acc = $global:kAccent
            $rows = [Math]::Max(3, [Console]::WindowHeight - 6)
            if ($sel -lt $top) { $top = $sel } elseif ($sel -ge $top + $rows) { $top = $sel - $rows + 1 }
            $sb = New-Object Text.StringBuilder
            [void]$sb.Append("$e[H$e[2J`r`n  $e[1;38;2;${acc}mKTerm keybinds$e[0m  $e[2m$view$e[0m`r`n`r`n")
            for ($i = $top; $i -lt [Math]::Min($items.Count, $top + $rows); $i++) {
                $it = $items[$i]
                $st = if (Get-KbState $it.id) { "$e[32mon $e[0m" } else { "$e[31moff$e[0m" }
                if ($i -eq $sel) {
                    [void]$sb.Append(("  $e[1;38;2;${acc}m> {0,-20}$e[0m $e[38;2;${acc}m<$e[0m$st$e[38;2;${acc}m>$e[0m  $e[2m{1}$e[0m`r`n" -f $it.name, $it.desc))
                } else {
                    [void]$sb.Append(("    $e[2m{0,-20}$e[0m  {1}   $e[2m{2}$e[0m`r`n" -f $it.name, $st, $it.desc))
                }
            }
            [void]$sb.Append("`r`n  $e[2mup/down select   left/right/enter toggle   r restart Explorer   q quit$e[0m  $note")
            [Console]::Write($sb.ToString())

            $size = "$([Console]::WindowWidth)x$([Console]::WindowHeight)"
            while (-not [Console]::KeyAvailable -and "$([Console]::WindowWidth)x$([Console]::WindowHeight)" -eq $size) { Start-Sleep -Milliseconds 40 }
            if (-not [Console]::KeyAvailable) { continue }
            $key = [Console]::ReadKey($true)
            $ch = [string]$key.KeyChar
            if ($key.Key -eq 'Escape' -or $ch -eq 'q') { return }
            if ($key.Key -eq 'UpArrow' -or $ch -eq 'k') { $sel = ($sel - 1 + $items.Count) % $items.Count; continue }
            if ($key.Key -eq 'DownArrow' -or $ch -eq 'j') { $sel = ($sel + 1) % $items.Count; continue }
            if ($ch -eq 'r') { Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue; $note = 'Explorer restarted'; continue }
            if ($key.Key -in 'LeftArrow', 'RightArrow', 'Enter', 'Spacebar' -or $ch -in 'h', 'l') {
                $it = $items[$sel]
                Set-KbState $it.id (-not (Get-KbState $it.id))
                $note = if (Get-KbChar $it.id) { 'press r to apply' } else { '' }
            }
        }
    } finally {
        [Console]::Write("$e[?25h$e[?1049l")
    }
}

# ---- `benchmark`: record kterm's own CPU / GPU / memory live, then print a summary -------------
# `benchmark --peak` draws in place in the scrollback instead of on its own page.

function global:benchmark {
    $procs = @(Get-Process kterm -ErrorAction SilentlyContinue)
    if (-not $procs) { 'benchmark: kterm is not running'; return }
    $peak = [bool]($args | Where-Object { $_ -in '--peak', '-peak', '--peek', '-peek' })
    $ids = $procs.Id
    $cores = [Environment]::ProcessorCount
    $spark = ' ' + -join (0x2581..0x2588 | ForEach-Object { [char]$_ })  # block bars; written as code points since this file has no BOM
    $cpuHist = New-Object Collections.Generic.List[double]
    $gpuHist = New-Object Collections.Generic.List[double]
    $memHist = New-Object Collections.Generic.List[double]
    $lastCpu = ($procs | ForEach-Object { $_.TotalProcessorTime.TotalSeconds } | Measure-Object -Sum).Sum
    $lastT = [Diagnostics.Stopwatch]::StartNew()
    $filter = ($ids | ForEach-Object { "Name LIKE 'pid_${_}_%engtype_3D'" }) -join ' OR '
    $line = {
        param($hist, $max)
        $w = [math]::Max(10, [Console]::WindowWidth - 30)
        $vals = $hist | Select-Object -Last $w
        -join ($vals | ForEach-Object { $spark[[int][math]::Min(8, [math]::Round($_ / [math]::Max($max, 1e-9) * 8))] })
    }

    # Inline mode redraws by moving back up over its previous 7 lines; the page mode just clears.
    $top = if ($peak) { '' } else { "$e[H$e[2J" }
    [Console]::Write($(if ($peak) { "$e[?25l" } else { "$e[?1049h$e[?25l" }))
    try {
        while ($true) {
            # Sample.
            $procs = @(Get-Process -Id $ids -ErrorAction SilentlyContinue)
            if (-not $procs) { break }
            $cpu = ($procs | ForEach-Object { $_.TotalProcessorTime.TotalSeconds } | Measure-Object -Sum).Sum
            $secs = $lastT.Elapsed.TotalSeconds; $lastT.Restart()
            $cpuHist.Add([math]::Max(0, ($cpu - $lastCpu) / $secs / $cores * 100)); $lastCpu = $cpu
            $gpu = try { (Get-CimInstance Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine -Filter $filter -ErrorAction Stop |
                    Measure-Object UtilizationPercentage -Sum).Sum } catch { 0 }
            $gpuHist.Add([math]::Min(100, [double]$gpu))
            $memHist.Add(($procs | Measure-Object WorkingSet64 -Sum).Sum / 1MB)

            # Draw.
            $acc = $global:kAccent
            $n = $cpuHist.Count
            $sb = New-Object Text.StringBuilder
            [void]$sb.Append("$top`r`n  $e[1;38;2;${acc}mKTerm benchmark$e[0m  $e[2m$n s recorded, pid $($ids -join ', ')$e[0m`r`n`r`n")
            foreach ($r in @(@('CPU', $cpuHist, '{0,6:0.0} %', 100), @('GPU', $gpuHist, '{0,6:0.0} %', 100), @('RAM', $memHist, '{0,6:0} MB', 0))) {
                $h = $r[1]
                $max = if ($r[3]) { [math]::Max(5, ($h | Measure-Object -Maximum).Maximum) } else { ($h | Measure-Object -Maximum).Maximum }
                [void]$sb.Append(("  $e[1m{0}$e[0m {1}  $e[38;2;${acc}m{2}$e[0m`r`n" -f $r[0], ($r[2] -f $h[$h.Count - 1]), (& $line $h $max)))
            }
            [void]$sb.Append("`r`n  $e[2mq quit and show summary$e[0m")
            [Console]::Write($sb.ToString())
            if ($peak) { $top = "`r$e[7F$e[J" }

            $until = (Get-Date).AddMilliseconds(1000 - $lastT.ElapsedMilliseconds)
            while ((Get-Date) -lt $until -and -not [Console]::KeyAvailable) { Start-Sleep -Milliseconds 40 }
            if ([Console]::KeyAvailable) {
                $k = [Console]::ReadKey($true)
                if ($k.Key -eq 'Escape' -or $k.KeyChar -eq 'q') { break }
            }
        }
    } finally {
        # Inline: wipe the key hint line and keep the graphs above the summary.
        [Console]::Write($(if ($peak) { "`r$e[K$e[?25h" } else { "$e[?25h$e[?1049l" }))
    }

    # Summary stays on the normal screen.
    $acc = $global:kAccent
    "  $e[1;38;2;${acc}mKTerm usage over $($cpuHist.Count) s$e[0m"
    foreach ($r in @(@('CPU', $cpuHist, '%'), @('GPU', $gpuHist, '%'), @('RAM', $memHist, 'MB'))) {
        if (-not $r[1].Count) { continue }
        $m = $r[1] | Measure-Object -Average -Minimum -Maximum
        '  {0}  avg {1,7:0.0} {4}   min {2,7:0.0} {4}   peak {3,7:0.0} {4}' -f $r[0], $m.Average, $m.Minimum, $m.Maximum, $r[2]
    }
}

# ---- `open <name>`: apps and games by name (`open discord`), or any file, folder or URL --------
# Run as a fresh window's first command, the window closes after launching; `--stay` keeps it.
function global:open {
    $stay = $args -contains '--stay'
    $q = "$($args | Where-Object { $_ -ne '--stay' })".Trim()
    if (-not $q) { 'usage: open <app, game, file, folder or url> [--stay]'; return }
    $ok = $true
    if ($q -match '^\w[\w+.-]*://') {
        Start-Process $q
    } elseif (Test-Path -LiteralPath $q) {
        Invoke-Item -LiteralPath $q
        $ok = $?
    } else {
        # Start menu apps: Win32, Store, and Steam/Riot/etc. game shortcuts.
        $apps = @(Get-StartApps | ForEach-Object { [pscustomobject]@{ Name = $_.Name; Target = "shell:AppsFolder\$($_.AppID)" } })
        # Epic games have no Start menu entry; the launcher's manifests list them.
        Get-ChildItem "$env:ProgramData\Epic\EpicGamesLauncher\Data\Manifests\*.item" -ErrorAction SilentlyContinue | ForEach-Object {
            try {
                $m = Get-Content $_.FullName -Raw | ConvertFrom-Json
                $apps += [pscustomobject]@{ Name = $m.DisplayName; Target = "com.epicgames.launcher://apps/$($m.CatalogNamespace)%3A$($m.CatalogItemId)%3A$($m.AppName)?action=launch&silent=true" }
            } catch {}
        }
        # Best match: exact name, then starts with, then contains (all case-insensitive).
        $hit = $apps | Where-Object { $_.Name -eq $q } | Select-Object -First 1
        if (-not $hit) { $hit = $apps | Where-Object { $_.Name.StartsWith($q, 'OrdinalIgnoreCase') } | Sort-Object { $_.Name.Length } | Select-Object -First 1 }
        if (-not $hit) { $hit = $apps | Where-Object { $_.Name.IndexOf($q, [StringComparison]::OrdinalIgnoreCase) -ge 0 } | Sort-Object { $_.Name.Length } | Select-Object -First 1 }
        if ($hit) {
            Start-Process explorer.exe "`"$($hit.Target)`""
            "Opening $($hit.Name)"
        } else {
            # Last resort: anything Windows can run by name (PATH, App Paths, e.g. `open notepad`).
            try { Start-Process $q -ErrorAction Stop } catch { "open: nothing called '$q'"; $ok = $false }
        }
    }
    # The running command joins the history only once it finishes, so no growth since the first
    # prompt means this is the first command.
    if ($ok -and -not $stay -and (Get-History).Count -eq $global:kHistBase) { exit }
}

# Running processes whose name or description contains $q (never this shell itself).
function global:Find-KtermProcs([string]$q) {
    $has = { param($s) $s -and $s.IndexOf($q, [StringComparison]::OrdinalIgnoreCase) -ge 0 }
    @(Get-Process | Where-Object { $_.Id -ne $PID -and ((& $has $_.ProcessName) -or (& $has $_.Description)) })
}

# ---- `close <name>`: end a running app. Lists matches and asks which; --all ends every match ----
function global:close {
    $all = $args -contains '--all'
    $q = "$($args | Where-Object { $_ -ne '--all' })".Trim()
    if (-not $q) { 'usage: close <name> [--all]'; return }
    $procs = Find-KtermProcs $q
    if (-not $procs) { "close: nothing running called '$q'"; return }

    # "Important": Windows' own programs, services, and processes too protected to show a path.
    $svcPids = @(Get-CimInstance Win32_Service -Filter "State='Running'" -ErrorAction SilentlyContinue | ForEach-Object ProcessId)
    # One entry per program: apps like Discord run several processes from the same exe.
    $apps = @($procs | Group-Object { "$($_.ProcessName)|$($_.Path)" } | ForEach-Object {
        $p = $_.Group[0]
        [pscustomobject]@{
            Name      = if ($p.Description) { $p.Description } else { $p.ProcessName }
            Exe       = $p.ProcessName
            Path      = $p.Path
            Procs     = $_.Group
            Important = -not $p.Path -or $p.Path.StartsWith($env:windir, 'OrdinalIgnoreCase') -or
                        @($_.Group | Where-Object { $svcPids -contains $_.Id }).Count -gt 0
        }
    } | Sort-Object Name)

    ''
    for ($i = 0; $i -lt $apps.Count; $i++) {
        $a = $apps[$i]
        $tag = if ($a.Important) { " $e[31m(system/service)$e[0m" } else { '' }
        $where = if ($a.Path) { $a.Path } else { '(path hidden: system or elevated process)' }
        "  {0,2}) $e[1m{1}$e[0m  $e[2m{2}.exe, {3} process(es)$e[0m{4}" -f ($i + 1), $a.Name, $a.Exe, $a.Procs.Count, $tag
        "      $e[2m$where$e[0m"
    }
    ''
    if ($all) {
        $pick = $apps
        if ($apps | Where-Object Important) {
            if ((Read-Host "  This includes system processes or services. Close all $($apps.Count) anyway? (y/N)") -notmatch '^\s*y') { 'Cancelled.'; return }
        }
    } else {
        $ans = Read-Host "  Which one? (1-$($apps.Count), Enter to cancel)"
        $n = 0
        if (-not [int]::TryParse($ans, [ref]$n) -or $n -lt 1 -or $n -gt $apps.Count) { 'Cancelled.'; return }
        $pick = @($apps[$n - 1])
    }

    foreach ($a in $pick) {
        $a.Procs | Stop-Process -Force -ErrorAction SilentlyContinue
        # Some processes exit on their own once their parent goes, so check what is really left.
        $left = @($a.Procs | Where-Object { Get-Process -Id $_.Id -ErrorAction SilentlyContinue })
        if ($left) { "  $e[31mCouldn't close $($a.Name)$e[0m (access denied? try an admin terminal)" } else { "  Closed $($a.Name)" }
    }
}

# ---- `admin`: reopen this terminal elevated (UAC prompt), in the same folder ------------------
$kAdminCwd = "$env:APPDATA\kterm\admin-cwd"
$kIsAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

function global:admin {
    if ($kIsAdmin) { 'admin: this terminal is already elevated'; return }
    $exe = (Get-Process kterm -ErrorAction SilentlyContinue | Select-Object -First 1).Path
    if (-not $exe) { 'admin: kterm is not running'; return }
    # Elevation drops the working directory, so hand it over through a file.
    [IO.File]::WriteAllText($kAdminCwd, $PWD.ProviderPath)
    try { Start-Process $exe '--admin' -Verb RunAs -ErrorAction Stop } catch { Remove-Item $kAdminCwd; 'admin: cancelled'; return }
    exit
}

if ($kIsAdmin -and (Test-Path $kAdminCwd)) {
    Set-Location -LiteralPath ([IO.File]::ReadAllText($kAdminCwd))
    Remove-Item $kAdminCwd
}

# ---- `hide <name>` / `show <name>`: send an app's windows to the tray (or nowhere) and back -----
# Like Medal's own "hide": the window and its taskbar button go away, the app keeps running, and
# its tray icon (if it has one) still opens it. `show` brings back what `hide` hid.
$kHiddenPath = "$env:APPDATA\kterm\hidden.json"

function global:Use-KtermWin {
    if ('KtermWin' -as [type]) { return }
    Add-Type @'
using System; using System.Collections.Generic; using System.Runtime.InteropServices;
public static class KtermWin {
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr h, uint cmd);
    [DllImport("user32.dll")] static extern int GetWindowTextLength(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    public static uint Pid(IntPtr h) { uint pid; GetWindowThreadProcessId(h, out pid); return pid; }
    // Visible, titled, unowned top-level windows of these processes: the ones with taskbar buttons.
    public static List<IntPtr> AppWindows(uint[] pids) {
        var set = new HashSet<uint>(pids); var found = new List<IntPtr>();
        EnumWindows((h, l) => {
            if (set.Contains(Pid(h)) && IsWindowVisible(h) && GetWindow(h, 4) == IntPtr.Zero && GetWindowTextLength(h) > 0) found.Add(h);
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@
}

function global:Get-KtermHidden {
    # ForEach unrolls the array: 5.1's ConvertFrom-Json emits a whole JSON array as one item.
    if (Test-Path $kHiddenPath) { try { return @(Get-Content $kHiddenPath -Raw | ConvertFrom-Json | ForEach-Object { $_ }) } catch {} }
    @()
}

function global:hide {
    $q = "$args".Trim()
    if (-not $q) { 'usage: hide <name>   (bring it back with: show <name>)'; return }
    Use-KtermWin
    $procs = Find-KtermProcs $q
    $wins = if ($procs) { [KtermWin]::AppWindows([uint32[]]@($procs | ForEach-Object Id)) }
    if (-not $wins) { "hide: no open window for '$q'"; return }
    $hidden = Get-KtermHidden
    foreach ($h in $wins) {
        $p = $procs | Where-Object Id -eq ([KtermWin]::Pid($h)) | Select-Object -First 1
        $name = if ($p.Description) { $p.Description } else { $p.ProcessName }
        [void][KtermWin]::ShowWindow($h, 0)  # SW_HIDE
        $hidden += [pscustomobject]@{ hwnd = [int64]$h; name = $name; exe = $p.ProcessName }
        "  Hid $name"
    }
    # WriteAllText: no BOM, same as config.json.
    [IO.File]::WriteAllText($kHiddenPath, (ConvertTo-Json @($hidden)))
    "  $e[2mBring it back with: show $q$e[0m"
}

function global:show {
    $q = "$args".Trim()
    Use-KtermWin
    $hidden = @(Get-KtermHidden | Where-Object { [KtermWin]::IsWindow([IntPtr]$_.hwnd) })  # drop closed ones
    $has = { param($s) -not $q -or ($s -and $s.IndexOf($q, [StringComparison]::OrdinalIgnoreCase) -ge 0) }
    $pick = @($hidden | Where-Object { (& $has $_.name) -or (& $has $_.exe) })
    if (-not $pick) {
        if ($hidden) { "show: nothing hidden matches '$q'. Hidden: $(($hidden | ForEach-Object name | Sort-Object -Unique) -join ', ')" }
        else { 'show: nothing is hidden' }
    } else {
        foreach ($w in $pick) {
            [void][KtermWin]::ShowWindow([IntPtr]$w.hwnd, 5)  # SW_SHOW
            [void][KtermWin]::SetForegroundWindow([IntPtr]$w.hwnd)
            "  Showing $($w.name)"
        }
    }
    [IO.File]::WriteAllText($kHiddenPath, (ConvertTo-Json @($hidden | Where-Object { $pick -notcontains $_ })))
}

# ---- `fetch <url>`: download what a link points to -------------------------------------------
# Video/audio sites (YouTube, TikTok, ...) go through yt-dlp; anything else (CDN files, Discord
# attachments, web pages) is saved as-is with curl. Risky-looking links ask before downloading.
$kMediaHosts = '(^|\.)(youtube\.com|youtu\.be|tiktok\.com|twitter\.com|x\.com|instagram\.com|facebook\.com|fb\.watch|' +
    'twitch\.tv|kick\.com|vimeo\.com|reddit\.com|redd\.it|soundcloud\.com|bandcamp\.com|dailymotion\.com|bilibili\.com|' +
    'streamable\.com|medal\.tv|rumble\.com|nicovideo\.jp)$'
$kRiskyExt = '\.(exe|msi|msix|appx|scr|com|pif|bat|cmd|ps1|psm1|vbs|vbe|js|jse|wsf|wsh|hta|jar|lnk|dll|cpl|reg|iso|img|vhdx?|application|appref-ms)$'

# Reasons a link looks dangerous; empty when nothing stands out.
function global:Test-KtermUrlRisk([string]$url, [string]$type) {
    $r = @()
    try { $u = [Uri]$url } catch { return @('not a valid link') }
    if ($u.Scheme -ne 'https') { $r += "not encrypted ($($u.Scheme)://), anyone on the network can tamper with it" }
    if ($u.UserInfo) { $r += "the part before '@' is a disguise; it really goes to $($u.Host)" }
    if ($u.HostNameType -in 'IPv4', 'IPv6') { $r += 'points at a raw IP address instead of a domain' }
    if ($u.Host -match '(^|\.)xn--') { $r += "lookalike-letters domain ($($u.IdnHost)) that may imitate a real site" }
    $file = [Uri]::UnescapeDataString($u.AbsolutePath)
    if ($file -match $kRiskyExt) { $r += "downloads a program or script ($($Matches[0]))" }
    if ($file -match '\.(png|jpe?g|gif|webp|pdf|docx?|txt|mp[34])\.\w+$') { $r += 'double file extension, a classic disguise' }
    if ($type -match 'x-msdownload|x-msdos-program|x-msi|x-sh|x-bat|java-archive') { $r += "server says it's an executable ($type)" }
    $r
}

function global:fetch {
    param(
        [Parameter(Position = 0)][string]$Url,
        [string]$Out,           # file or folder; default: current folder
        [string]$Quality,       # best (default), worst, or a height: 1080, 720p, ...
        [switch]$Audio,         # audio only
        [string]$AudioTrack,    # audio language, e.g. en, es, ja (multi-language YouTube videos)
        [string]$Format,        # container: mp4 (default) / mkv / webm, or with -Audio: mp3 (default) / m4a / opus / wav / flac
        [switch]$IgnoreWarning, # don't ask before risky links
        [Parameter(ValueFromRemainingArguments)][string[]]$Rest  # extra yt-dlp flags, e.g. --embed-subs
    )
    if (-not $Url) {
        'usage: fetch <url> [-Out path] [-Quality best|worst|1080] [-Audio] [-AudioTrack en] [-Format mp4|mp3|...] [-IgnoreWarning] [yt-dlp flags]'
        return
    }
    if ($Url -notmatch '^\w[\w+.-]*://') { $Url = "https://$Url" }

    # Follow redirects first (short links, CDN hops) so the checks see where it really goes.
    $info = @(curl.exe -sIL -o NUL -m 15 -A 'Mozilla/5.0' -w '%{url_effective}\n%{content_type}\n%header{content-disposition}' $Url)
    $final = if ($info[0]) { $info[0] } else { $Url }
    $type = "$($info[1])"
    $risks = @(Test-KtermUrlRisk $Url $type) + @(if ($final -ne $Url) { Test-KtermUrlRisk $final $type }) | Select-Object -Unique
    if ($risks) {
        "  $e[1;31mWarning:$e[0m this link looks risky"
        if ($final -ne $Url) { "  $e[2mit leads to $final$e[0m" }
        $risks | ForEach-Object { "   - $_" }
        if (-not $IgnoreWarning -and (Read-Host '  Download anyway? (y/N)') -notmatch '^\s*y') { 'Cancelled.'; return }
    }

    $media = $Audio -or $Quality -or $AudioTrack -or $Rest -or ([Uri]$final).Host -match $kMediaHosts
    $dir = $PWD.ProviderPath
    $name = $null
    if ($Out) {
        $Out = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Out)
        if ((Test-Path -LiteralPath $Out -PathType Container) -or $Out -match '[\\/]$') { $dir = $Out }
        else { $dir = Split-Path $Out; $name = Split-Path $Out -Leaf }
        [void](New-Item -ItemType Directory -Force $dir)
    }

    if ($media) {
        $yt = (Get-Command yt-dlp -ErrorAction SilentlyContinue).Source
        if (-not $yt) { $yt = "$env:LOCALAPPDATA\Microsoft\WinGet\Links\yt-dlp.exe" }
        if (-not (Test-Path $yt)) {
            if ((Read-Host '  Video downloads need yt-dlp (and ffmpeg). Install them with winget now? (Y/n)') -match '^\s*n') { return }
            winget install -e --id yt-dlp.yt-dlp --accept-source-agreements --accept-package-agreements
            winget install -e --id yt-dlp.FFmpeg --accept-source-agreements --accept-package-agreements
            if (-not (Test-Path $yt)) { 'fetch: yt-dlp install failed'; return }
        }
        $h = if ($Quality -match '^(\d+)p?$') { "[height<=$($Matches[1])]" } else { '' }
        $a = if ($AudioTrack) { "ba[language^=$AudioTrack]/ba" } else { 'ba' }
        $f = if ($Audio) { $a }
             elseif ($Quality -eq 'worst') { 'wv*+wa/w' }
             else { "bv*$h+$($a -replace '/ba$', '')/bv*$h+ba/b$h/b" }
        $ytArgs = @('-f', $f, '-P', $dir, '-o', $(if ($name) { $name } else { '%(title)s [%(id)s].%(ext)s' }))
        $ytArgs += if ($Audio) { '-x', '--audio-format', $(if ($Format) { $Format } else { 'mp3' }) }
                   else { '--merge-output-format', $(if ($Format) { $Format } else { 'mp4' }) }
        if ($IgnoreWarning) { $ytArgs += '--no-warnings' }
        & $yt @ytArgs @Rest $Url
        return
    }

    # Plain file or page: name it after the server's suggestion, else the link's last path part.
    if (-not $name -and "$($info[2])" -match 'filename\*?=(?:UTF-8'''')?"?([^";]+)') { $name = [Uri]::UnescapeDataString($Matches[1]) }
    if (-not $name) { $name = [Uri]::UnescapeDataString(([Uri]$final).Segments[-1]).Trim('/') }
    if (-not $name) { $name = ([Uri]$final).Host }
    if ($type -match 'text/html' -and $name -notmatch '\.html?$') { $name += '.html' }
    $name = $name -replace '[<>:"/\\|?*]', '_'
    $path = Join-Path $dir $name
    curl.exe -L --fail -# -A 'Mozilla/5.0' -o $path $Url
    if ($LASTEXITCODE) { "fetch: download failed (curl exit $LASTEXITCODE)"; return }
    # Mark of the Web, like a browser download: Windows SmartScreen checks it before running it.
    Set-Content -LiteralPath $path -Stream Zone.Identifier -Value "[ZoneTransfer]`r`nZoneId=3`r`nHostUrl=$final"
    "  Saved $path ({0:N0} KB)" -f ((Get-Item -LiteralPath $path).Length / 1KB)
}

# ---- Arch-style prompt:  user@host ~/path >  (accent colored, red after a failure) -----------
function global:prompt {
    $ok = $?
    # History before the first command (kterm's startup -Command counts as an entry); see `open`.
    if ($null -eq $global:kHistBase) { $global:kHistBase = (Get-History).Count }
    $acc = if ($global:kAccent) { "38;2;$global:kAccent" } else { '34' }
    $user = $env:USERNAME  # kterm stars it on screen when hide_user is on
    $p = $PWD.ProviderPath
    if ($p.StartsWith($HOME, [StringComparison]::OrdinalIgnoreCase)) { $p = '~' + $p.Substring($HOME.Length) }
    $p = $p -replace '\\', '/'
    $Host.UI.RawUI.WindowTitle = $p  # names this terminal in KTerm's tray menu
    $arrow = if ($ok) { $acc } else { '31' }
    "$e[1;${acc}m$user@$($env:COMPUTERNAME.ToLower())$e[0m $e[1m$p$e[0m $e[${arrow}m$([char]0x276F)$e[0m "
}
