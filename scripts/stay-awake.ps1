Add-Type -AssemblyName System.Windows.Forms

$sig = @'
[System.Runtime.InteropServices.DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint esFlags);
[System.Runtime.InteropServices.DllImport("user32.dll")]   public static extern bool SetCursorPos(int X, int Y);
[System.Runtime.InteropServices.DllImport("user32.dll")]   public static extern bool GetCursorPos(out POINT lpPoint);
[System.Runtime.InteropServices.StructLayout(System.Runtime.InteropServices.LayoutKind.Sequential)] public struct POINT { public int X; public int Y; }
'@
if (-not ('Win32.PSStayApi' -as [type])) {
    Add-Type -MemberDefinition $sig -Name PSStayApi -Namespace Win32 | Out-Null
}

# ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED | ES_AWAYMODE_REQUIRED
$flags = [Convert]::ToUInt32('80000043', 16)
[Win32.PSStayApi]::SetThreadExecutionState($flags) | Out-Null

while ($true) {
    [Win32.PSStayApi]::SetThreadExecutionState($flags) | Out-Null

    $p = New-Object Win32.PSStayApi+POINT
    [void][Win32.PSStayApi]::GetCursorPos([ref]$p)
    [void][Win32.PSStayApi]::SetCursorPos($p.X + 1, $p.Y)
    Start-Sleep -Milliseconds 50
    [void][Win32.PSStayApi]::SetCursorPos($p.X, $p.Y)

    [System.Windows.Forms.SendKeys]::SendWait('{F15}')

    Start-Sleep -Seconds 50
}
