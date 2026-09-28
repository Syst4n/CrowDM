# Measures memory of a running CrowDM and all of its child processes
# (WebView2 browser, renderer, GPU and utility processes).
# Usage: powershell -File tools/bench/memory.ps1 [-Label text]
param([string]$Label = "")

$app = Get-Process CrowDM -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $app) { Write-Error "CrowDM is not running"; exit 1 }
$all = Get-CimInstance Win32_Process
$ids = @($app.Id)
for ($i = 0; $i -lt 5; $i++) {
  $ids += ($all | Where-Object { $ids -contains $_.ParentProcessId } | ForEach-Object ProcessId)
  $ids = @($ids | Select-Object -Unique)
}
$procs = Get-Process -Id $ids -ErrorAction SilentlyContinue
$private = ($procs | Measure-Object PrivateMemorySize64 -Sum).Sum
$ws = ($procs | Measure-Object WorkingSet64 -Sum).Sum
"{0}: {1} processes, private {2:N1} MB, working set {3:N1} MB (CrowDM.exe alone: private {4:N1} MB)" -f $Label, $procs.Count, ($private / 1MB), ($ws / 1MB), ($app.PrivateMemorySize64 / 1MB)
