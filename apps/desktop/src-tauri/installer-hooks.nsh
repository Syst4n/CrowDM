; CrowDM NSIS installer hooks (included by the Tauri NSIS template).

!macro NSIS_HOOK_PREINSTALL
  ; A browser may be running the native messaging host; it holds the file open.
  nsExec::Exec 'taskkill /IM crowdm-host.exe /F'
!macroend

!macro NSIS_HOOK_POSTINSTALL
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec 'taskkill /IM crowdm-host.exe /F'
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Browser integration and autostart entries written by the app (per user).
  DeleteRegKey HKCU "Software\Google\Chrome\NativeMessagingHosts\com.crowdm.host"
  DeleteRegKey HKCU "Software\Microsoft\Edge\NativeMessagingHosts\com.crowdm.host"
  DeleteRegKey HKCU "Software\Chromium\NativeMessagingHosts\com.crowdm.host"
  DeleteRegKey HKCU "Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.crowdm.host"
  DeleteRegKey HKCU "Software\Mozilla\NativeMessagingHosts\com.crowdm.host"
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "CrowDM"
  DeleteRegKey HKCU "Software\CrowDM"
!macroend
