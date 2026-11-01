#Requires -Version 5.1
<#
.SYNOPSIS
    tidl Outlook COM bridge -- syncs the local Outlook calendar to a local tidl CalDAV endpoint.

.DESCRIPTION
    Intended to run every 5-15 minutes via Windows Task Scheduler.
    Reads Outlook calendar items in a rolling window, builds iCalendar
    payloads, and syncs to localhost tidl via CalDAV (PUT new/changed,
    DELETE removed). Skips gracefully if Outlook is not running.

    Config: bridge.conf in the same directory as this script.
    Log:    %APPDATA%\tidl-bridge\bridge.log  (rotated at ~1 MB)

.EXAMPLE
    powershell.exe -ExecutionPolicy Bypass -NonInteractive -WindowStyle Hidden -File bridge.ps1
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# -- Config --------------------------------------------------------------------

$ScriptDir  = Split-Path -Parent $MyInvocation.MyCommand.Path
$ConfigPath = Join-Path $ScriptDir 'bridge.conf'
$LogDir     = Join-Path $env:APPDATA 'tidl-bridge'
$LogPath    = Join-Path $LogDir 'bridge.log'

function Read-Config {
    if (-not (Test-Path $ConfigPath)) {
        throw "Config file not found: $ConfigPath"
    }
    $conf = @{}
    Get-Content $ConfigPath | ForEach-Object {
        if ($_ -match '^\s*([^#=\s]+)\s*=\s*(.+?)\s*$') {
            $conf[$Matches[1]] = $Matches[2]
        }
    }
    foreach ($key in @('TIDL_URL', 'TIDL_USER', 'TIDL_TOKEN', 'TIDL_CALENDAR')) {
        if (-not $conf.ContainsKey($key)) {
            throw "Missing required config key: $key"
        }
    }
    if (-not $conf.ContainsKey('OUTLOOK_CALENDAR'))   { $conf['OUTLOOK_CALENDAR']   = 'Calendar' }
    if (-not $conf.ContainsKey('WINDOW_PAST_DAYS'))   { $conf['WINDOW_PAST_DAYS']   = '30' }
    if (-not $conf.ContainsKey('WINDOW_FUTURE_DAYS')) { $conf['WINDOW_FUTURE_DAYS'] = '180' }
    return $conf
}

# -- Logging -------------------------------------------------------------------

function Write-Log {
    param([string]$Level, [string]$Message)
    if (-not (Test-Path $LogDir)) { New-Item -ItemType Directory -Path $LogDir | Out-Null }
    # Rotate log if > 1 MB
    if ((Test-Path $LogPath) -and (Get-Item $LogPath).Length -gt 1MB) {
        Move-Item $LogPath "$LogPath.old" -Force
    }
    $line = "[$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')] [$Level] $Message"
    Add-Content -Path $LogPath -Value $line
    if ($Level -eq 'ERROR') { Write-Warning $line }
}

# -- iCalendar helpers ---------------------------------------------------------

function Format-ICalDateTimeUtc {
    param([datetime]$dt)
    return $dt.ToUniversalTime().ToString('yyyyMMddTHHmmssZ')
}

# tidl stores DTSTART/DTEND as naive local wall-clock time (its own event
# creation UI uses an HTML datetime-local input with no timezone conversion,
# and its ICS parser strips a trailing 'Z' and stores the digits verbatim
# instead of converting to local time). So DTSTART/DTEND must be emitted as
# Outlook's local wall-clock time with no 'Z' -- converting to genuine UTC
# here would make tidl display events shifted by the local UTC offset.
# DTSTAMP/LAST-MODIFIED are unaffected -- those stay real UTC per RFC 5545,
# they're metadata, not the user-visible event time.
function Format-ICalDateTimeFloating {
    param([datetime]$dt)
    return $dt.ToString('yyyyMMddTHHmmss')
}

function Escape-ICalText {
    param([string]$text)
    if ([string]::IsNullOrEmpty($text)) { return '' }
    # RFC 5545 section 3.3.11 TEXT escaping
    $text = $text -replace '\\',  '\\'
    $text = $text -replace ';',   '\;'
    $text = $text -replace ',',   '\,'
    $text = $text -replace "`r`n", '\n'
    $text = $text -replace "`n",   '\n'
    $text = $text -replace "`r",   '\n'
    return $text
}

function Fold-ICalLine {
    # RFC 5545 section 3.1: fold at 75 octets, continuation line begins with a space
    param([string]$line)
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($line)
    if ($bytes.Length -le 75) { return $line }

    $result = [System.Text.StringBuilder]::new()
    $col = 0
    $buf = [System.Text.StringBuilder]::new()

    foreach ($b in $bytes) {
        if ($col -eq 75) {
            $result.Append($buf.ToString()) | Out-Null
            $result.Append("`r`n ") | Out-Null
            $buf.Clear() | Out-Null
            $col = 1  # the leading space counts as 1
        }
        $buf.Append([char]$b) | Out-Null
        $col++
    }
    $result.Append($buf.ToString()) | Out-Null
    return $result.ToString()
}

function Build-VEvent {
    param(
        [string]  $Uid,
        [string]  $Summary,
        [datetime]$Start,
        [datetime]$End,
        [bool]    $IsAllDay,
        [string]  $Location,
        [string]  $Description,
        [datetime]$LastModified
    )

    $dtstamp    = Format-ICalDateTimeUtc (Get-Date)
    $lastModUtc = Format-ICalDateTimeUtc $LastModified

    $lines = [System.Collections.Generic.List[string]]::new()
    $lines.Add('BEGIN:VCALENDAR')
    $lines.Add('VERSION:2.0')
    $lines.Add('PRODID:-//tidl-bridge//Outlook COM Bridge//EN')
    $lines.Add('CALSCALE:GREGORIAN')
    $lines.Add('BEGIN:VEVENT')
    $lines.Add("UID:$Uid")
    $lines.Add("DTSTAMP:$dtstamp")
    $lines.Add("LAST-MODIFIED:$lastModUtc")
    $lines.Add((Fold-ICalLine "SUMMARY:$(Escape-ICalText $Summary)"))

    if ($IsAllDay) {
        # Outlook all-day: Start = 00:00 on start day, End = 00:00 on day *after* last day.
        # DTEND;VALUE=DATE is exclusive, matching Outlook's End value directly.
        $lines.Add("DTSTART;VALUE=DATE:$($Start.ToString('yyyyMMdd'))")
        $lines.Add("DTEND;VALUE=DATE:$($End.ToString('yyyyMMdd'))")
    } else {
        $lines.Add("DTSTART:$(Format-ICalDateTimeFloating $Start)")
        $lines.Add("DTEND:$(Format-ICalDateTimeFloating $End)")
    }

    if (-not [string]::IsNullOrWhiteSpace($Location)) {
        $lines.Add((Fold-ICalLine "LOCATION:$(Escape-ICalText $Location)"))
    }
    if (-not [string]::IsNullOrWhiteSpace($Description)) {
        # Truncate very long bodies (Outlook can store entire email threads)
        $body = $Description
        if ($body.Length -gt 4000) { $body = $body.Substring(0, 4000) + '...' }
        $lines.Add((Fold-ICalLine "DESCRIPTION:$(Escape-ICalText $body)"))
    }

    $lines.Add('END:VEVENT')
    $lines.Add('END:VCALENDAR')

    return ($lines -join "`r`n") + "`r`n"
}

# -- HTTP / CalDAV helpers -----------------------------------------------------

function Get-BasicAuthHeader {
    param([string]$User, [string]$Token)
    $encoded = [Convert]::ToBase64String(
        [System.Text.Encoding]::UTF8.GetBytes("${User}:${Token}")
    )
    return "Basic $encoded"
}

function Invoke-CalDAV {
    # Windows PowerShell 5.1's Invoke-WebRequest/-RestMethod type -Method as
    # Microsoft.PowerShell.Commands.WebRequestMethod, a closed enum with no
    # PROPFIND member (WebDAV custom verbs need -CustomMethod, added in PS6+).
    # HttpWebRequest.Method is a plain string, so it accepts any verb on 5.1.
    # Body/ContentType are deliberately untyped -- see the ExistingEtag comment
    # in Send-PutEvent for why a [string] = $null default silently becomes ''
    # instead of staying $null (which broke the "$null -ne $Body" checks below
    # for Send-DeleteEvent, which omits both and relied on the $null default).
    param(
        [string]   $Method,
        [string]   $Uri,
        [hashtable]$Headers,
        $Body        = $null,
        $ContentType = $null,
        [int[]]    $SuccessCodes = @(200, 201, 204, 207)
    )

    $request = [System.Net.HttpWebRequest]::Create($Uri)
    $request.Method = $Method
    $request.UserAgent = 'tidl-bridge/1.0'

    foreach ($key in $Headers.Keys) {
        switch ($key) {
            'Authorization' { $request.Headers['Authorization'] = $Headers[$key] }
            'User-Agent'    { $request.UserAgent = $Headers[$key] }
            default         { $request.Headers.Add($key, $Headers[$key]) }
        }
    }
    if ($null -ne $ContentType) {
        $request.ContentType = $ContentType
    }

    if ($null -ne $Body) {
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($Body)
        $request.ContentLength = $bytes.Length
        $stream = $request.GetRequestStream()
        try { $stream.Write($bytes, 0, $bytes.Length) } finally { $stream.Close() }
    } elseif ($Method -eq 'PUT' -or $Method -eq 'POST') {
        $request.ContentLength = 0
    }

    # Headers are only dumped on failure (folded into the thrown message), not
    # for every request -- at ~700 events/run, logging headers unconditionally
    # drowns the log and makes the actual failures harder to find.
    # Authorization is redacted so the token doesn't end up in bridge.log.
    try {
        $response = $request.GetResponse()
    } catch [System.Net.WebException] {
        $headerDump = $request.Headers.ToString() -replace '\r?\n', ' | ' -replace '(Authorization:\s*)\S+', '$1<redacted>'
        $resp = $_.Exception.Response
        if ($null -ne $resp) {
            $status = [int]$resp.StatusCode
            # 404 on DELETE = already gone, treat as success
            if ($Method -eq 'DELETE' -and $status -eq 404) { return $null }
            throw "HTTP $status on $Method $Uri (request headers: $headerDump)"
        }
        throw "$_ (request headers: $headerDump)"
    }

    try {
        $reader = New-Object System.IO.StreamReader($response.GetResponseStream())
        $content = $reader.ReadToEnd()
        $reader.Close()
    } finally {
        $response.Close()
    }
    return [pscustomobject]@{ Content = $content; StatusCode = [int]$response.StatusCode }
}

# Returns hashtable: uid -> etag for all .ics resources in the calendar collection
function Get-TidlEvents {
    param([string]$CalUrl, [hashtable]$BaseHeaders)

    $body = @'
<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:">
  <D:prop>
    <D:getetag/>
  </D:prop>
</D:propfind>
'@
    $hdrs = $BaseHeaders.Clone()
    $hdrs['Depth'] = '1'

    $response = Invoke-CalDAV -Method 'PROPFIND' -Uri $CalUrl -Headers $hdrs `
        -Body $body -ContentType 'application/xml; charset=utf-8'

    # Temporary diagnostic -- remove once the empty-existing-set bug is confirmed fixed.
    Write-Log 'INFO' "PROPFIND HTTP $($response.StatusCode), $($response.Content.Length) bytes: $($response.Content.Substring(0, [Math]::Min(500, $response.Content.Length)))"

    $doc = New-Object System.Xml.XmlDocument
    $doc.LoadXml($response.Content)
    $ns = New-Object System.Xml.XmlNamespaceManager($doc.NameTable)
    $ns.AddNamespace('D', 'DAV:')

    $responseNodes = $doc.SelectNodes('//D:response', $ns)
    Write-Log 'INFO' "PROPFIND parsed $($responseNodes.Count) //D:response node(s)"

    $result = @{}
    foreach ($resp in $responseNodes) {
        $hrefNode = $resp.SelectSingleNode('D:href', $ns)
        $etagNode = $resp.SelectSingleNode('D:propstat/D:prop/D:getetag', $ns)
        $href = if ($hrefNode) { $hrefNode.InnerText.Trim() } else { $null }
        $etag = if ($etagNode) { $etagNode.InnerText.Trim() } else { $null }
        if ([string]::IsNullOrEmpty($href) -or [string]::IsNullOrEmpty($etag)) { continue }
        if ($href.EndsWith('/')) { continue }  # skip the collection itself

        $fileName = $href.Split('/')[-1]
        $uid = $fileName -replace '\.ics$', ''
        if ($uid) { $result[$uid] = $etag }
    }
    return $result
}

function Send-PutEvent {
    # ExistingEtag is deliberately untyped. A [string] constraint here would
    # coerce an explicitly-passed $null into '' during parameter binding
    # ([AllowNull()] does NOT prevent this -- that attribute only applies to
    # mandatory parameters, per Microsoft's own docs, and this one isn't),
    # so "$null -ne $ExistingEtag" below would always be true and every PUT
    # would send If-Match: '' instead of If-None-Match: *.
    param(
        [string]   $EventUrl,
        [hashtable]$BaseHeaders,
        [string]   $ICalData,
        $ExistingEtag = $null  # $null means new resource
    )
    $hdrs = $BaseHeaders.Clone()
    if ($null -ne $ExistingEtag) {
        $hdrs['If-Match'] = $ExistingEtag
    } else {
        $hdrs['If-None-Match'] = '*'
    }
    Invoke-CalDAV -Method 'PUT' -Uri $EventUrl -Headers $hdrs `
        -Body $ICalData -ContentType 'text/calendar; charset=utf-8' | Out-Null
}

function Send-DeleteEvent {
    param(
        [string]   $EventUrl,
        [hashtable]$BaseHeaders,
        [string]   $Etag
    )
    $hdrs = $BaseHeaders.Clone()
    $hdrs['If-Match'] = $Etag
    Invoke-CalDAV -Method 'DELETE' -Uri $EventUrl -Headers $hdrs | Out-Null
}

# -- Outlook reading -----------------------------------------------------------

function Get-OutlookEvents {
    param(
        [string]$CalendarName,
        [int]   $PastDays,
        [int]   $FutureDays
    )

    # Attach to the already-running Outlook process (does not launch Outlook)
    try {
        $outlook = [Runtime.InteropServices.Marshal]::GetActiveObject('Outlook.Application')
    } catch [System.Runtime.InteropServices.COMException] {
        Write-Log 'INFO' 'Outlook is not running -- skipping sync'
        return $null
    }

    $namespace = $outlook.GetNamespace('MAPI')

    # olFolderCalendar = 9
    $defaultCal = $namespace.GetDefaultFolder(9)
    $calFolder  = $null

    if ($defaultCal.Name -eq $CalendarName) {
        $calFolder = $defaultCal
    } else {
        foreach ($folder in $defaultCal.Folders) {
            if ($folder.Name -eq $CalendarName) { $calFolder = $folder; break }
        }
    }

    if ($null -eq $calFolder) {
        throw "Outlook calendar folder not found: '$CalendarName'. " +
              "Available folders under default calendar: " +
              (($defaultCal.Folders | ForEach-Object { $_.Name }) -join ', ')
    }

    $windowStart = (Get-Date).AddDays(-$PastDays).Date
    $windowEnd   = (Get-Date).AddDays($FutureDays).Date

    # IncludeRecurrences must be set BEFORE Sort() and before Restrict()
    $items = $calFolder.Items
    $items.IncludeRecurrences = $true
    $items.Sort('[Start]')

    # Outlook date filter format: MM/dd/yyyy HH:mm
    $filter = "[Start] >= '$($windowStart.ToString('MM/dd/yyyy HH:mm'))'" +
              " AND [Start] < '$($windowEnd.ToString('MM/dd/yyyy HH:mm'))'"
    $restricted = $items.Restrict($filter)

    $events = [System.Collections.Generic.List[hashtable]]::new()

    foreach ($item in $restricted) {
        # olAppointment = 26
        if ($item.Class -ne 26) { continue }

        # OlRecurrenceState values (per Microsoft's Outlook object model):
        #   0 = olApptNotRecurring
        #   1 = olApptMaster       (series master -- excluded when IncludeRecurrences is set)
        #   2 = olApptOccurrence   (individual occurrence of a series)
        #   3 = olApptException    (modified occurrence)
        $recState = $item.RecurrenceState

        # GlobalAppointmentID is the same for all occurrences of a series.
        # Append the precise start timestamp so each occurrence gets a unique UID.
        $uid = if ($recState -eq 2 -or $recState -eq 3) {
            "$($item.GlobalAppointmentID)_$($item.Start.ToString('yyyyMMddTHHmmss'))"
        } else {
            $item.GlobalAppointmentID
        }

        $events.Add(@{
            Uid          = $uid
            Summary      = [string]$item.Subject
            Start        = [datetime]$item.Start
            End          = [datetime]$item.End
            IsAllDay     = [bool]$item.AllDayEvent
            Location     = [string]$item.Location
            Description  = [string]$item.Body
            LastModified = [datetime]$item.LastModificationTime
        })
    }

    Write-Log 'INFO' "Read $($events.Count) events from Outlook ('$CalendarName', window: -${PastDays}d..+${FutureDays}d)"
    return $events
}

# -- Main ----------------------------------------------------------------------

function Main {
    Write-Log 'INFO' 'Bridge starting'

    $conf     = Read-Config
    $baseUrl  = $conf['TIDL_URL'].TrimEnd('/')
    $user     = $conf['TIDL_USER']
    $token    = $conf['TIDL_TOKEN']
    $calendar = $conf['TIDL_CALENDAR']
    $pastDays = [int]$conf['WINDOW_PAST_DAYS']
    $futDays  = [int]$conf['WINDOW_FUTURE_DAYS']
    $outCal   = $conf['OUTLOOK_CALENDAR']

    $calUrl     = "$baseUrl/caldav/$user/$calendar/"
    $authHeader = Get-BasicAuthHeader $user $token
    $baseHeaders = @{
        Authorization = $authHeader
        'User-Agent'  = 'tidl-bridge/1.0'
    }

    # 1. Read Outlook
    $outlookEvents = Get-OutlookEvents $outCal $pastDays $futDays
    if ($null -eq $outlookEvents) {
        Write-Log 'INFO' 'Bridge done (Outlook not running)'
        return
    }

    # 2. Build uid -> ical map
    $outlookMap = @{}
    foreach ($ev in $outlookEvents) {
        $ical = Build-VEvent `
            -Uid          $ev.Uid `
            -Summary      $ev.Summary `
            -Start        $ev.Start `
            -End          $ev.End `
            -IsAllDay     $ev.IsAllDay `
            -Location     $ev.Location `
            -Description  $ev.Description `
            -LastModified $ev.LastModified
        $outlookMap[$ev.Uid] = $ical
    }
    if ($outlookMap.Count -ne $outlookEvents.Count) {
        Write-Log 'WARN' "$($outlookEvents.Count) Outlook items collapsed to $($outlookMap.Count) unique UIDs -- check the RecurrenceState mapping in Get-OutlookEvents if this gap is large"
    }

    # 3. Fetch existing events from tidl
    $existing = Get-TidlEvents $calUrl $baseHeaders
    Write-Log 'INFO' "tidl has $($existing.Count) existing events in '$calendar'"

    $putCount    = 0
    $deleteCount = 0
    $errCount    = 0

    # 4. PUT: new events + updates to existing ones
    foreach ($uid in $outlookMap.Keys) {
        $ical     = $outlookMap[$uid]
        $eventUrl = "$baseUrl/caldav/$user/$calendar/$([Uri]::EscapeDataString($uid)).ics"
        try {
            $existingEtag = if ($existing.ContainsKey($uid)) { $existing[$uid] } else { $null }
            Send-PutEvent $eventUrl $baseHeaders $ical $existingEtag
            $putCount++
        } catch {
            Write-Log 'ERROR' "PUT failed [$uid]: $_"
            $errCount++
        }
    }

    # 5. DELETE: events in tidl that are no longer in the Outlook window
    foreach ($uid in $existing.Keys) {
        if ($outlookMap.ContainsKey($uid)) { continue }
        $eventUrl = "$baseUrl/caldav/$user/$calendar/$([Uri]::EscapeDataString($uid)).ics"
        try {
            Send-DeleteEvent $eventUrl $baseHeaders $existing[$uid]
            $deleteCount++
        } catch {
            Write-Log 'ERROR' "DELETE failed [$uid]: $_"
            $errCount++
        }
    }

    Write-Log 'INFO' "Bridge done: $putCount PUT, $deleteCount DELETE, $errCount errors"
}

try {
    Main
} catch {
    Write-Log 'ERROR' "Fatal: $_"
    exit 1
}
