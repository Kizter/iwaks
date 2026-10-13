# Generate a corpus of tiny valid WAV files for the NFR A2 scan benchmark
# (docs/design.md: "scan 10,000 files < 30 s").
#
# Every file is a minimal 16-bit mono PCM WAV (44-byte header + 400 bytes of
# silence), so the benchmark measures scanner throughput, not disk volume:
# 10,000 files and their database land well under 10 MB.
#
# Usage:
#   powershell -File scripts\gen-scan-bench-corpus.ps1 [-Root <dir>] [-Count <n>]

param(
  [string]$Root = (Join-Path $env:TEMP "opencode\iwaks-scan-bench-10k"),
  [int]$Count = 10000
)

$dataLen = 400
$buf = New-Object byte[] (44 + $dataLen)
[Text.Encoding]::ASCII.GetBytes("RIFF").CopyTo($buf, 0)
[BitConverter]::GetBytes([int](36 + $dataLen)).CopyTo($buf, 4)
[Text.Encoding]::ASCII.GetBytes("WAVE").CopyTo($buf, 8)
[Text.Encoding]::ASCII.GetBytes("fmt ").CopyTo($buf, 12)
[BitConverter]::GetBytes([int]16).CopyTo($buf, 16)          # fmt chunk size
[BitConverter]::GetBytes([int16]1).CopyTo($buf, 20)         # PCM
[BitConverter]::GetBytes([int16]1).CopyTo($buf, 22)         # mono
[BitConverter]::GetBytes([int]44100).CopyTo($buf, 24)       # sample rate
[BitConverter]::GetBytes([int]88200).CopyTo($buf, 28)       # byte rate
[BitConverter]::GetBytes([int16]2).CopyTo($buf, 32)         # block align
[BitConverter]::GetBytes([int16]16).CopyTo($buf, 34)        # bits per sample
[Text.Encoding]::ASCII.GetBytes("data").CopyTo($buf, 36)
[BitConverter]::GetBytes([int]$dataLen).CopyTo($buf, 40)    # silent samples

New-Item -ItemType Directory -Force -Path $Root | Out-Null
$sw = [System.Diagnostics.Stopwatch]::StartNew()
for ($i = 0; $i -lt $Count; $i++) {
  [System.IO.File]::WriteAllBytes((Join-Path $Root ("t{0:D5}.wav" -f $i)), $buf)
}
$sw.Stop()
Write-Output ("wrote {0} files in {1:N2}s -> {2}" -f $Count, $sw.Elapsed.TotalSeconds, $Root)