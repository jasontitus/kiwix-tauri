; NSIS hooks for the Windows installer (bundle > windows > nsis > installerHooks).
;
; "Look up in Kiwix": register kiwix://search links for this install. The
; command passes the link as the only argument after --open-url; the app
; accepts --open-url only as the whole command line, and only for
; kiwix://search?q=…, which can do nothing but prefill a library search.
;
; Never take over a kiwix: handler that belongs to another application (for
; example Kiwix Desktop), whether it is registered per user or machine-wide:
; HKCR is the merged view Windows uses to launch links. Write the key only
; when no kiwix: class exists at all or it already runs this install, and
; remove it on uninstall only in the same case.

!define KIWIX_URL_KEY "Software\Classes\kiwix"
!define KIWIX_URL_COMMAND '"$INSTDIR\${MAINBINARYNAME}.exe" --open-url "%1"'

!macro NSIS_HOOK_POSTINSTALL
  ClearErrors
  EnumRegKey $R1 HKCR "kiwix" 0
  ${If} ${Errors}
    StrCpy $R0 ""
    StrCpy $R2 "absent"
  ${Else}
    ReadRegStr $R0 HKCR "kiwix\shell\open\command" ""
    StrCpy $R2 "present"
  ${EndIf}
  ${If} $R2 == "absent"
  ${OrIf} $R0 == '${KIWIX_URL_COMMAND}'
    WriteRegStr SHCTX "${KIWIX_URL_KEY}" "" "URL:Kiwix search"
    WriteRegStr SHCTX "${KIWIX_URL_KEY}" "URL Protocol" ""
    WriteRegStr SHCTX "${KIWIX_URL_KEY}\DefaultIcon" "" '"$INSTDIR\${MAINBINARYNAME}.exe",0'
    WriteRegStr SHCTX "${KIWIX_URL_KEY}\shell\open\command" "" '${KIWIX_URL_COMMAND}'
  ${EndIf}
!macroend

; After the uninstall has passed its "Kiwix is running" check, which can
; still abort it: a cancelled uninstall must keep working links.
!macro NSIS_HOOK_POSTUNINSTALL
  ; An update reinstalls into the same folder and rewrites the key; keep it.
  ${If} $UpdateMode <> 1
    ReadRegStr $R0 SHCTX "${KIWIX_URL_KEY}\shell\open\command" ""
    ${If} $R0 == '${KIWIX_URL_COMMAND}'
      DeleteRegKey SHCTX "${KIWIX_URL_KEY}"
    ${EndIf}
  ${EndIf}
!macroend
