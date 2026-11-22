$edge = "C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
$out = $PSScriptRoot
$base = ([Uri](Join-Path (Split-Path $out -Parent) "index.html")).AbsoluteUri
$states = [ordered]@{
  "hist"     = "vue=hist"
  "cookies"  = "vue=cookies&auto=1"
  "ferme"    = "ouvert=0"
}
$extra = [ordered]@{
  "1-cookies-off"    = "d=1&vue=cookies"
  "1-filtre"         = "d=1&q=steam"
  "1-confirm"        = "d=1&confirm=hist"
  "2-cookies-confirm"= "d=2&vue=cookies&auto=1&mode=accepter&confirm=cookies"
  "3-exception"      = "d=3&vue=cookies&auto=1&exception=1"
  "2-vide"           = "d=2&vide=1"
  "3-auto-replie"    = "d=3&auto=1"
}
$jobs = @()
foreach ($d in 1..3) { foreach ($e in $states.GetEnumerator()) { $jobs += ,@("$d-$($e.Key)", "d=$d&$($e.Value)") } }
foreach ($e in $extra.GetEnumerator()) { $jobs += ,@($e.Key, $e.Value) }
foreach ($j in $jobs) {
  $p = Join-Path $out ($j[0] + ".png")
  & $edge --headless=new --disable-gpu --hide-scrollbars --screenshot="$p" --window-size=1280,896 "$base`?$($j[1])" 2>$null | Out-Null
}
Get-ChildItem $out -Filter *.png | Select-Object Name, Length
