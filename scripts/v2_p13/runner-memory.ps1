param([ValidateRange(1,16)][int]$Workers = 2, [Parameter(Mandatory=$true)][string]$OutputDirectory)
$ErrorActionPreference='Stop'
$taskRoot=(Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$taskOutput=[IO.Path]::GetFullPath($OutputDirectory)
if(-not $taskOutput.StartsWith((Join-Path $taskRoot '.factory\v2\p13\'),[StringComparison]::OrdinalIgnoreCase)){throw 'Use a project-local P13 evidence directory'}
New-Item -ItemType Directory -Path $taskOutput -Force | Out-Null
if(Test-Path -LiteralPath (Join-Path $taskOutput 'results.json')){throw 'Preserve prior evidence'}
$taskFrontend=Join-Path $taskRoot 'apps/dashboard'
$taskOs=Get-CimInstance Win32_OperatingSystem
$taskResult=[ordered]@{schema=1;workers=$Workers;startedAt=[DateTime]::UtcNow.ToString('o');availableLogicalCpus=[Environment]::ProcessorCount;ramBytes=([long]$taskOs.TotalVisibleMemorySize*1024);freeBeforeBytes=([long]$taskOs.FreePhysicalMemory*1024);samples=@();scope='Sum of owned test process working sets and private committed bytes. Shared pages may be counted more than once. One-second polling can miss short peaks.'}
if($Workers -gt 4 -and $taskResult.freeBeforeBytes -lt 4294967296){$taskResult.status='NOT_RUN_LOW_MEMORY_HEADROOM';$taskResult.reason='A guarded unrestricted-worker attempt requires 4 GiB available physical memory. This is an unexecuted stress test, not a pass.'}
else{
 $taskNode=(Get-Command node).Source
 $taskExitFile=Join-Path $taskOutput 'child-exit.json'
 $taskArguments=@(('"'+(Join-Path $PSScriptRoot 'run-vitest.mjs')+'"'),[string]$Workers,('"'+$taskExitFile+'"'))
 $taskProcess=Start-Process -FilePath $taskNode -ArgumentList $taskArguments -WorkingDirectory $taskFrontend -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $taskOutput 'stdout.log') -RedirectStandardError (Join-Path $taskOutput 'stderr.log')
 $taskPid=$taskProcess.Id
 $taskResult.pid=$taskPid
 while(-not $taskProcess.HasExited){
  $taskRows=Get-CimInstance Win32_Process -Property ProcessId,ParentProcessId,WorkingSetSize,PageFileUsage
  $taskIds=[Collections.Generic.HashSet[uint32]]::new();[void]$taskIds.Add($taskPid)
  do{$taskAdded=$false;foreach($taskRow in $taskRows){if($taskIds.Contains([uint32]$taskRow.ParentProcessId)-and $taskIds.Add([uint32]$taskRow.ProcessId)){$taskAdded=$true}}}while($taskAdded)
  $taskOwned=@($taskRows|Where-Object{$taskIds.Contains([uint32]$_.ProcessId)})
  $taskSample=[ordered]@{at=[DateTime]::UtcNow.ToString('o');processes=$taskOwned.Count;workingSetBytes=([long]($taskOwned|Measure-Object WorkingSetSize -Sum).Sum);privateCommittedBytes=([long]($taskOwned|Measure-Object PageFileUsage -Sum).Sum*1024);freePhysicalBytes=([long](Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory*1024)}
  $taskResult.samples+=,[pscustomobject]$taskSample
  $taskResult|ConvertTo-Json -Depth 6|Set-Content -LiteralPath (Join-Path $taskOutput 'samples.partial.json') -Encoding UTF8
  Start-Sleep -Milliseconds 1000
  $taskProcess.Refresh()
 }
 $taskProcess.WaitForExit()
 $taskChildExit=Get-Content -LiteralPath $taskExitFile -Raw|ConvertFrom-Json
 $taskResult.childExit=$taskChildExit
 $taskResult.exit=$taskChildExit.code
 $taskResult.status=if($null -ne $taskChildExit.code -and $taskChildExit.code -eq 0){'PASS'}else{'FAILED'}
 $taskResult.peakWorkingSetBytes=($taskResult.samples|Measure-Object workingSetBytes -Maximum).Maximum
 $taskResult.peakPrivateCommittedBytes=($taskResult.samples|Measure-Object privateCommittedBytes -Maximum).Maximum
}
$taskResult.finishedAt=[DateTime]::UtcNow.ToString('o')
$taskResult|ConvertTo-Json -Depth 6|Set-Content -LiteralPath (Join-Path $taskOutput 'results.json') -Encoding UTF8
Write-Output "$($taskResult.status) workers=$Workers output=$taskOutput"
if($taskResult.status -eq 'FAILED'){exit 1}
