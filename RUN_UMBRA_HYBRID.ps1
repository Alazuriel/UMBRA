[CmdletBinding()]
param(
 [ValidateSet('reference','balanced','gpu-feed','throughput')][string]$Mode='gpu-feed',
 [ValidateSet('wide','copies')][string]$CopyMode='wide',
 [ValidateSet('4k','1440p','1080p','900p','720p')][string]$Resolution='720p',
 [ValidateSet('series-x','series-s')][string]$Console='series-x',
 [ValidateSet('warm','lazy','off')][string]$CacheMode='warm',
 [ValidateRange(1,64)][int]$Jobs=6,
 [string]$GameExe='F:\GTA6.Xbox.v1.0.2.1-CYBERLEEK\game_scarlett_master.exe',
 [switch]$NoHud,
 [switch]$Capped,
 [switch]$TextureNoWait,
 [ValidateSet('safe','overlap')][string]$GpuSchedule='safe'
)
$ErrorActionPreference='Stop'
$root=$PSScriptRoot
$exe=Join-Path $root 'target\release\victory.exe'
if (!(Test-Path -LiteralPath $exe)) {
 throw "Missing Hybrid release build. In $root run: cargo build --release -p victory"
}
if (!(Test-Path -LiteralPath $GameExe)) { throw "Game executable missing: $GameExe" }
$reset=@(
 'VICTORY_RELAY','VICTORY_LATE_WAIT','VICTORY_LATE_WAITS','VICTORY_CARD_BUFFERS',
 'VICTORY_CARD_BUFFERS_MIN_MB','VICTORY_CARD_BUFFERS_MB','VICTORY_CARD_WINDOWS_MB',
 'VICTORY_D3D12_WARP','VICTORY_D3D12_DEBUG','VICTORY_D3D12_GBV','VICTORY_D3D12_DRED',
 'VICTORY_INPLACE','VICTORY_WAVE_WIDE','VICTORY_SHADER_OPT','VICTORY_SKIP_PIPELINES',
 'VICTORY_BVH_BUILDER','VICTORY_BVH_QUARANTINE','VICTORY_RT_RESOLVE',
 'VICTORY_TEXTURE_NO_WAIT','VICTORY_TEXTURE_COPY','VICTORY_JOIN_COPIES','VICTORY_WIDE_BACK',
 'VICTORY_SHADER_SAVE','VICTORY_CAPTURE','VICTORY_CAPTURE_WORLD','VICTORY_CAPTURE_FRAMES',
 'UMBRA_V021_RELAXED_F16','UMBRA_V021_DESCRIPTOR_ZERO','UMBRA_V025_RTBVH_PREWARM',
 'UMBRA_V024_IDLE_MS','UMBRA_V024_BVH_REPORT','UMBRA_V022_TEXTURE_MB','UMBRA_V022_TEXTURE_MS',
 'UMBRA_V022_TEXTURE_SLOTS','UMBRA_V027_GPU_720_OUTPUT','UMBRA_V027_INTRO_HOLD',
 'UMBRA_HUD','UMBRA_HUD_CORNER','UMBRA_FPS_LIMIT','UMBRA_V025_DSTORAGE_WORKERS',
 'UMBRA_V025_PIPELINE_WORKERS','VICTORY_DISPLAY_MODE','VICTORY_GAME_DIR'
)
if($GpuSchedule -eq 'overlap') {
 # GPU timing disables late submissions; retained snapshots disable the relay.
 $reset+=@('VICTORY_GPU_TIMING','VICTORY_KEEP_SNAPSHOTS')
}
$old=@{}
foreach($key in $reset) { $old[$key]=[Environment]::GetEnvironmentVariable($key,'Process') }
try {
 foreach($key in $reset) { [Environment]::SetEnvironmentVariable($key,$null,'Process') }
 $sizes=@{'4k'='3840x2160';'1440p'='2560x1440';'1080p'='1920x1080';'900p'='1600x900';'720p'='1280x720'}
 $env:VICTORY_DISPLAY_MODE=$sizes[$Resolution]
 $env:VICTORY_GAME_DIR=Split-Path -Parent $GameExe
 $env:VICTORY_JOIN_COPIES='1'
 if($TextureNoWait) { $env:VICTORY_TEXTURE_NO_WAIT='1' }
 if($CopyMode -eq 'wide') { $env:VICTORY_WIDE_BACK='1' }
 if($GpuSchedule -eq 'overlap') {
  $env:VICTORY_LATE_WAIT='3'
  $env:VICTORY_RELAY='2'
 }
 $env:VICTORY_BVH_BUILDER='0'
 $env:VICTORY_RT_RESOLVE='0'
 $env:VICTORY_BVH_QUARANTINE='1'
 $env:VICTORY_WAVE_WIDE='0'
 $env:UMBRA_FPS_LIMIT=if($Capped){'1'}else{'0'}
 $env:UMBRA_HUD=if($NoHud){'0'}else{'1'}
 $env:UMBRA_HUD_CORNER='left'
 $plans=@{'reference'=@(12,6);'balanced'=@(8,6);'gpu-feed'=@(6,4);'throughput'=@(10,6)}
 $env:UMBRA_V025_DSTORAGE_WORKERS=[string]$plans[$Mode][0]
 $env:UMBRA_V025_PIPELINE_WORKERS=[string]$plans[$Mode][1]
 Write-Host "UMBRA AMD HYBRID DEXNOM v0.2 | $Mode | $CopyMode | $Resolution display | $Console | RT off | GPU schedule=$GpuSchedule | cache=$CacheMode | texture-no-wait=$($TextureNoWait.IsPresent)"
 Push-Location $root
 try {
  & py -3 (Join-Path $root 'umbra_launcher\launcher.py') --runtime $exe --game-exe $GameExe --console $Console --cache-mode $CacheMode --jobs $Jobs
  if($LASTEXITCODE -ne 0) { throw "Shader launcher exited with code $LASTEXITCODE" }
 } finally { Pop-Location }
} finally {
 foreach($key in $reset) { [Environment]::SetEnvironmentVariable($key,$old[$key],'Process') }
}
