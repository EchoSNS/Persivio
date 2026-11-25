; The look of the Windows installer and uninstaller (ADR-027).
;
; Tauri includes this file (`installerHooks`) after NSIS's page macros and before it uses
; them. The page macros are replaced here; everything that installs, updates or removes
; files stays Tauri's. If an upgrade of Tauri or NSIS renames one, `!macroundef` stops the
; build instead of shipping a half-styled installer.
;
; Silent (/S) and passive (/P, used by the updater) runs never see the first and last page.

!define PV_BG "0A1019"
!define PV_FIELD "141C29"
!define PV_TEXT "EAF0F7"
!define PV_MUTED "A3ADBD"
; Window and drawing colors: 0x00BBGGRR for Windows, 0xAARRGGBB for GDI+.
!define PV_BG_REF 0x0019100A
!define PV_TEXT_REF 0x00F7F0EA
!define PV_MUTED_REF 0x00BDADA3
!define PV_BG_ARGB 0xFF0A1019
!define PV_TEAL 0xFF0F766E
!define PV_TEAL_HOT 0xFF12877E
!define PV_GREY 0xFF212936
!define PV_GREY_HOT 0xFF2D3747
!define PV_FOCUS 0xFF2DD4BF
!define PV_WIDTH 560
!define PV_HEIGHT 400
!define PV_DIR "${__FILEDIR__}"

Var PvDpi
Var PvFontTitle
Var PvFontBody
Var PvFontSmall
Var PvPage
Var PvPrimary
Var PvBackHandle
Var PvPageBack
Var PvFolder
Var PvDesktopCheck
Var PvDesktop
Var PvStartCheck
Var PvOldVersion
Var PvSkipReinstall
Var PvX
Var PvY
Var PvW
Var PvH

!define MUI_CUSTOMFUNCTION_GUIINIT PvGuiInit
!define MUI_CUSTOMFUNCTION_UNGUIINIT un.PvGuiInit

; ${Px} $out 96dpi-pixels
!define Px "!insertmacro PV_PX "
!macro PV_PX OUT VALUE
  IntOp ${OUT} ${VALUE} * $PvDpi
  IntOp ${OUT} ${OUT} / 96
!macroend

!macro PV_RECT X Y W H
  ${Px} $PvX ${X}
  ${Px} $PvY ${Y}
  ${Px} $PvW ${W}
  ${Px} $PvH ${H}
!macroend

; Leaves the control's handle on the stack.
!macro PV_LABEL X Y W H TEXT FONT COLOR BACK
  !insertmacro PV_RECT ${X} ${Y} ${W} ${H}
  ; Not NSD_CreateLabel: those are transparent, and the background can end up over them.
  nsDialogs::CreateControl STATIC ${DEFAULT_STYLES} 0 $PvX $PvY $PvW $PvH "${TEXT}"
  Exch $0
  SendMessage $0 ${WM_SETFONT} ${FONT} 1
  SetCtlColors $0 "${COLOR}" "${BACK}"
  Exch $0
!macroend

!macro PV_BUTTON X Y W H TEXT UN CLICK
  !insertmacro PV_RECT ${X} ${Y} ${W} ${H}
  ${NSD_CreateButton} $PvX $PvY $PvW $PvH "${TEXT}"
  Exch $0
  SendMessage $0 ${WM_SETFONT} $PvFontBody 1
  ${NSD_OnClick} $0 ${CLICK}
  ${NSD_OnNotify} $0 ${UN}PvDrawButton
  Exch $0
!macroend

!macro PV_CHECKBOX X Y W H TEXT UN
  !insertmacro PV_RECT ${X} ${Y} ${W} ${H}
  ${NSD_CreateCheckBox} $PvX $PvY $PvW $PvH "${TEXT}"
  Exch $0
  SendMessage $0 ${WM_SETFONT} $PvFontBody 1
  ${NSD_OnNotify} $0 ${UN}PvDrawCheckbox
  Exch $0
!macroend

; GDI+: a rounded rectangle path in $R4 from $5,$6 (position), $7,$8 (size), $R5 (diameter).
!macro PV_ROUND_PATH
  System::Call 'gdiplus::GdipCreatePath(i 0, *p .R4)'
  IntOp $R6 $5 + $7
  IntOp $R6 $R6 - $R5
  IntOp $R7 $6 + $8
  IntOp $R7 $R7 - $R5
  ; Angles are 32-bit floats: 180, 270, 0 and 90 degrees, each a quarter turn.
  System::Call 'gdiplus::GdipAddPathArcI(p R4, i r5, i r6, i R5, i R5, i 0x43340000, i 0x42B40000)'
  System::Call 'gdiplus::GdipAddPathArcI(p R4, i R6, i r6, i R5, i R5, i 0x43870000, i 0x42B40000)'
  System::Call 'gdiplus::GdipAddPathArcI(p R4, i R6, i R7, i R5, i R5, i 0, i 0x42B40000)'
  System::Call 'gdiplus::GdipAddPathArcI(p R4, i r5, i R7, i R5, i R5, i 0x42B40000, i 0x42B40000)'
  System::Call 'gdiplus::GdipClosePathFigure(p R4)'
!macroend

; Functions the installer and the uninstaller both need (NSIS wants two copies).
!macro PV_FUNCTIONS UN
  Function ${UN}PvGuiInit
    InitPluginsDir
    System::Call 'user32::GetDpiForWindow(p $HWNDPARENT) i .r0'
    StrCpy $PvDpi $0
    ${If} $PvDpi == "error"
    ${OrIf} $PvDpi < 96
      StrCpy $PvDpi 96
    ${EndIf}
    ; The background contains the logo; the larger one is for high-resolution screens.
    ${If} $PvDpi > 96
      File "/oname=$PLUGINSDIR\pv-background.bmp" "${PV_DIR}\background-2x.bmp"
    ${Else}
      File "/oname=$PLUGINSDIR\pv-background.bmp" "${PV_DIR}\background.bmp"
    ${EndIf}

    System::Call '*(i 1, p 0, i 0, i 0) p .r0'
    System::Call 'gdiplus::GdiplusStartup(*p .r1, p r0, p 0)'
    System::Free $0

    CreateFont $PvFontTitle "Segoe UI Semibold" "19" "600"
    CreateFont $PvFontBody "Segoe UI" "10" "400"
    CreateFont $PvFontSmall "Segoe UI" "9" "400"

    ; The title bar takes the page's color (Windows 11) or at least turns dark (10).
    System::Call 'dwmapi::DwmSetWindowAttribute(p $HWNDPARENT, i 20, *i 1, i 4)'
    System::Call 'dwmapi::DwmSetWindowAttribute(p $HWNDPARENT, i 35, *i ${PV_BG_REF}, i 4)'
    System::Call 'dwmapi::DwmSetWindowAttribute(p $HWNDPARENT, i 36, *i ${PV_TEXT_REF}, i 4)'
    SetCtlColors $HWNDPARENT "${PV_TEXT}" "${PV_BG}"

    ; Window size: our client size plus the frame.
    System::Call 'user32::GetWindowRect(p $HWNDPARENT, @ r0)'
    System::Call '*$0(i .r1, i .r2, i .r3, i .r4)'
    System::Call 'user32::GetClientRect(p $HWNDPARENT, @ r0)'
    System::Call '*$0(i, i, i .r5, i .r6)'
    ${Px} $PvW ${PV_WIDTH}
    ${Px} $PvH ${PV_HEIGHT}
    IntOp $7 $3 - $1
    IntOp $7 $7 - $5
    IntOp $7 $7 + $PvW
    IntOp $8 $4 - $2
    IntOp $8 $8 - $6
    IntOp $8 $8 + $PvH
    System::Call 'user32::GetSystemMetrics(i 0) i .r3'
    System::Call 'user32::GetSystemMetrics(i 1) i .r4'
    IntOp $1 $3 - $7
    IntOp $1 $1 / 2
    IntOp $2 $4 - $8
    IntOp $2 $2 / 2
    System::Call 'user32::SetWindowPos(p $HWNDPARENT, p 0, i r1, i r2, i r7, i r8, i 0x14)'

    ; The page fills the window; NSIS's own header, lines and buttons are not used.
    GetDlgItem $0 $HWNDPARENT 1018
    System::Call 'user32::MoveWindow(p r0, i 0, i 0, i $PvW, i $PvH, i 0)'
    !insertmacro PV_HIDE 1
    !insertmacro PV_HIDE 2
    !insertmacro PV_HIDE 3
    !insertmacro PV_HIDE 1028
    !insertmacro PV_HIDE 1034
    !insertmacro PV_HIDE 1035
    !insertmacro PV_HIDE 1036
    !insertmacro PV_HIDE 1037
    !insertmacro PV_HIDE 1038
    !insertmacro PV_HIDE 1039
    !insertmacro PV_HIDE 1045
    !insertmacro PV_HIDE 1046
    !insertmacro PV_HIDE 1256

    System::Call 'user32::LoadImageW(p 0, w "$PLUGINSDIR\pv-background.bmp", i 0, i $PvW, i $PvH, i 0x10) p .r1'
    StrCpy $PvBackHandle $1
  FunctionEnd

  ; The background with a title and subtitle ($R8, $R9) painted into it: labels would have
  ; to be transparent, and Windows sometimes paints the picture over those.
  Function ${UN}PvTitle
    System::Call 'user32::CopyImage(p $PvBackHandle, i 0, i 0, i 0, i 0) p .s'
    Pop $PvPageBack
    System::Call 'gdi32::CreateCompatibleDC(p 0) p .r0'
    System::Call 'gdi32::SelectObject(p r0, p $PvPageBack) p .r1'
    System::Call 'gdi32::SetBkMode(p r0, i 1)'
    ${Px} $3 124
    ${Px} $5 540
    ${Px} $4 32
    System::Call 'gdi32::SelectObject(p r0, p $PvFontTitle) p .r2'
    System::Call 'gdi32::SetTextColor(p r0, i ${PV_TEXT_REF})'
    ${If} $R9 == ""
      ${Px} $6 100
    ${Else}
      ${Px} $6 70
    ${EndIf}
    System::Call '*(i r3, i r4, i r5, i r6) p .r8'
    System::Call 'user32::DrawTextW(p r0, w R8, i -1, p r8, i 0x24)' ; one line, middle
    System::Free $8
    System::Call 'gdi32::SelectObject(p r0, p $PvFontBody)'
    System::Call 'gdi32::SetTextColor(p r0, i ${PV_MUTED_REF})'
    ${Px} $4 72
    ${Px} $6 120
    System::Call '*(i r3, i r4, i r5, i r6) p .r8'
    System::Call 'user32::DrawTextW(p r0, w R9, i -1, p r8, i 0x10)' ; wraps
    System::Free $8
    System::Call 'gdi32::SelectObject(p r0, p r2)'
    System::Call 'gdi32::SelectObject(p r0, p r1)'
    System::Call 'gdi32::DeleteDC(p r0)'
  FunctionEnd

  ; Starts a page: the title and subtitle in $R8 and $R9.
  Function ${UN}PvBegin
    !insertmacro PV_HIDE 2 ; left over from the progress page
    nsDialogs::Create 1018
    Pop $PvPage
    SetCtlColors $PvPage "${PV_TEXT}" "${PV_BG}"

    Call ${UN}PvTitle
    ; For screen readers.
    !insertmacro PV_LABEL 0 0 1 1 "$R8. $R9" $PvFontSmall "${PV_BG}" "${PV_BG}"
    Pop $0
  FunctionEnd

  ; The background comes last: controls created earlier are above it.
  Function ${UN}PvShow
    ${NSD_CreateBitmap} 0 0 100% 100% ""
    Pop $0
    SendMessage $0 0x0172 0 $PvPageBack ; STM_SETIMAGE, IMAGE_BITMAP
    nsDialogs::Show
  FunctionEnd

  Function ${UN}PvNext
    Pop $0
    System::Call 'user32::PostMessageW(p $HWNDPARENT, i 0x408, p 1, p 0)'
  FunctionEnd

  Function ${UN}PvCancel
    Pop $0
    System::Call 'user32::PostMessageW(p $HWNDPARENT, i 0x111, p 2, p 0)'
  FunctionEnd

  ; NM_CUSTOMDRAW of a button. In: handle, code, pointer to NMCUSTOMDRAW.
  Function ${UN}PvDrawButton
    System::Store S
    Pop $R0
    Pop $R1
    Pop $R2
    ${If} $R1 <> -12
      System::Store L
      Return
    ${EndIf}
    System::Call '*$R2(p, p, i, i .r3, p .r4, i .r5, i .r6, i .r7, i .r8, p, i .r9)'
    ${If} $3 = 1
    ${OrIf} $3 = 3
      ; $9: 1 pressed, 4 disabled, 0x10 focus, 0x40 hot
      IntOp $1 $9 & 0x41
      ${If} $R0 = $PvPrimary
        StrCpy $2 ${PV_TEAL}
        ${If} $1 <> 0
          StrCpy $2 ${PV_TEAL_HOT}
        ${EndIf}
      ${Else}
        StrCpy $2 ${PV_GREY}
        ${If} $1 <> 0
          StrCpy $2 ${PV_GREY_HOT}
        ${EndIf}
      ${EndIf}
      System::Call 'gdiplus::GdipCreateFromHDC(p r4, *p .R3)'
      System::Call 'gdiplus::GdipSetSmoothingMode(p R3, i 4)'
      System::Call 'gdiplus::GdipGraphicsClear(p R3, i ${PV_BG_ARGB})'
      IntOp $7 $7 - 1
      IntOp $8 $8 - 1
      ${Px} $R5 20
      !insertmacro PV_ROUND_PATH
      System::Call 'gdiplus::GdipCreateSolidFill(i r2, *p .R8)'
      System::Call 'gdiplus::GdipFillPath(p R3, p R8, p R4)'
      System::Call 'gdiplus::GdipDeleteBrush(p R8)'
      IntOp $1 $9 & 0x10
      ${If} $1 <> 0
        System::Call 'gdiplus::GdipCreatePen1(i ${PV_FOCUS}, i 0x40000000, i 2, *p .R8)'
        System::Call 'gdiplus::GdipDrawPath(p R3, p R8, p R4)'
        System::Call 'gdiplus::GdipDeletePen(p R8)'
      ${EndIf}
      System::Call 'gdiplus::GdipDeletePath(p R4)'
      System::Call 'gdiplus::GdipDeleteGraphics(p R3)'
      System::Call 'user32::GetWindowTextW(p R0, w .r1, i 200)'
      System::Call 'gdi32::SelectObject(p r4, p $PvFontBody) p .R8'
      System::Call 'gdi32::SetBkMode(p r4, i 1)'
      System::Call 'gdi32::SetTextColor(p r4, i ${PV_TEXT_REF})'
      IntOp $2 $R2 + 20
      System::Call 'user32::DrawTextW(p r4, w r1, i -1, p r2, i 0x25)'
      System::Call 'gdi32::SelectObject(p r4, p R8)'
    ${EndIf}
    System::Store L
    ${NSD_Return} 4 ; CDRF_SKIPDEFAULT
  FunctionEnd

  ; The same for a check box: a rounded box, then its text.
  Function ${UN}PvDrawCheckbox
    System::Store S
    Pop $R0
    Pop $R1
    Pop $R2
    ${If} $R1 <> -12
      System::Store L
      Return
    ${EndIf}
    System::Call '*$R2(p, p, i, i .r3, p .r4, i .r5, i .r6, i .r7, i .r8, p, i .r9)'
    ${If} $3 = 1
    ${OrIf} $3 = 3
      StrCpy $R9 $8 ; height
      SendMessage $R0 0x00F0 0 0 $1 ; BM_GETCHECK
      System::Call 'gdiplus::GdipCreateFromHDC(p r4, *p .R3)'
      System::Call 'gdiplus::GdipSetSmoothingMode(p R3, i 4)'
      System::Call 'gdiplus::GdipGraphicsClear(p R3, i ${PV_BG_ARGB})'
      ${Px} $7 17
      StrCpy $8 $7
      StrCpy $5 1
      IntOp $6 $R9 - $8
      IntOp $6 $6 / 2
      ${Px} $R5 9
      !insertmacro PV_ROUND_PATH
      StrCpy $2 ${PV_GREY}
      ${If} $1 = 1
        StrCpy $2 ${PV_TEAL}
      ${EndIf}
      System::Call 'gdiplus::GdipCreateSolidFill(i r2, *p .R8)'
      System::Call 'gdiplus::GdipFillPath(p R3, p R8, p R4)'
      System::Call 'gdiplus::GdipDeleteBrush(p R8)'
      StrCpy $2 0xFF6B7688
      IntOp $0 $9 & 0x10
      ${If} $0 <> 0
        StrCpy $2 ${PV_FOCUS}
      ${EndIf}
      ${If} $1 <> 1
      ${OrIf} $0 <> 0
        System::Call 'gdiplus::GdipCreatePen1(i r2, i 0x3F800000, i 2, *p .R8)'
        System::Call 'gdiplus::GdipDrawPath(p R3, p R8, p R4)'
        System::Call 'gdiplus::GdipDeletePen(p R8)'
      ${EndIf}
      ${If} $1 = 1
        System::Call 'gdiplus::GdipCreatePen1(i 0xFFFFFFFF, i 0x40000000, i 2, *p .R8)'
        ${Px} $R5 4
        ${Px} $R6 7
        ${Px} $R7 13
        ${Px} $R1 9
        ${Px} $2 5
        IntOp $R5 $R5 + 1
        IntOp $R6 $R6 + 1
        IntOp $R7 $R7 + 1
        IntOp $R1 $R1 + $6
        IntOp $2 $2 + $6
        ${Px} $3 12
        IntOp $3 $3 + $6
        System::Call 'gdiplus::GdipDrawLineI(p R3, p R8, i R5, i R1, i R6, i r3)'
        System::Call 'gdiplus::GdipDrawLineI(p R3, p R8, i R6, i r3, i R7, i r2)'
        System::Call 'gdiplus::GdipDeletePen(p R8)'
      ${EndIf}
      System::Call 'gdiplus::GdipDeletePath(p R4)'
      System::Call 'gdiplus::GdipDeleteGraphics(p R3)'
      System::Call 'user32::GetWindowTextW(p R0, w .r1, i 200)'
      System::Call 'gdi32::SelectObject(p r4, p $PvFontBody) p .R8'
      System::Call 'gdi32::SetBkMode(p r4, i 1)'
      System::Call 'gdi32::SetTextColor(p r4, i ${PV_TEXT_REF})'
      ${Px} $0 28
      System::Call '*$R2(p, p, i, i, p, i, i, i .r7, i .r8)'
      System::Call '*(i r0, i 0, i r7, i r8) p .r2'
      System::Call 'user32::DrawTextW(p r4, w r1, i -1, p r2, i 0x24)'
      System::Free $2
      System::Call 'gdi32::SelectObject(p r4, p R8)'
    ${EndIf}
    System::Store L
    ${NSD_Return} 4
  FunctionEnd

  ; The progress page is NSIS's own dialog: restyle its controls. Title in $R8.
  Function ${UN}PvStyleProgress
    FindWindow $PvPage "#32770" "" $HWNDPARENT
    SetCtlColors $PvPage "${PV_TEXT}" "${PV_BG}"
    ${Px} $PvW ${PV_WIDTH}
    ${Px} $PvH ${PV_HEIGHT}
    ; Shorter than the window: NSIS's button below would be covered.
    ${Px} $0 325
    System::Call 'user32::MoveWindow(p $PvPage, i 0, i 0, i $PvW, i r0, i 0)'
    GetDlgItem $0 $PvPage 1016 ; the log
    ShowWindow $0 ${SW_HIDE}
    GetDlgItem $0 $PvPage 1027 ; "Show details"
    System::Call 'user32::MoveWindow(p r0, i -3000, i 0, i 1, i 1, i 0)'

    !insertmacro PV_RECT 40 232 480 22
    GetDlgItem $0 $PvPage 1006
    SetCtlColors $0 "${PV_MUTED}" "${PV_BG}"
    SendMessage $0 ${WM_SETFONT} $PvFontSmall 1
    System::Call 'user32::MoveWindow(p r0, i $PvX, i $PvY, i $PvW, i $PvH, i 1)'

    !insertmacro PV_RECT 40 214 480 6
    GetDlgItem $0 $PvPage 1004
    System::Call 'uxtheme::SetWindowTheme(p r0, w " ", w " ")'
    System::Call 'user32::SetWindowLongW(p r0, i -20, i 0)' ; no sunken edge
    System::Call 'user32::GetWindowLongW(p r0, i -16) i .r1'
    IntOp $1 $1 & 0xFF7FFFFF ; no border
    IntOp $1 $1 | 1 ; smooth
    System::Call 'user32::SetWindowLongW(p r0, i -16, i r1)'
    SendMessage $0 0x0409 0 0x00BFD42D ; PBM_SETBARCOLOR
    SendMessage $0 0x2001 0 0x00362921 ; PBM_SETBKCOLOR
    System::Call 'user32::SetWindowPos(p r0, p 0, i $PvX, i $PvY, i $PvW, i $PvH, i 0x24)'

    StrCpy $R9 ""
    Call ${UN}PvTitle
    System::Call 'user32::SetWindowTextW(p $HWNDPARENT, w R8)' ; for screen readers
    ${Px} $PvW ${PV_WIDTH}
    ${Px} $PvH ${PV_HEIGHT}
    System::Call 'user32::CreateWindowExW(i 0, w "STATIC", w "", i 0x5400000E, i 0, i 0, i $PvW, i $PvH, p $PvPage, p 0, p 0, p 0) p .r0'
    SendMessage $0 0x0172 0 $PvPageBack
    System::Call 'user32::SetWindowPos(p r0, p 1, i 0, i 0, i 0, i 0, i 0x13)'

    ; NSIS's own button: it becomes usable when the installation fails.
    GetDlgItem $0 $HWNDPARENT 2
    !insertmacro PV_RECT 400 340 120 36
    System::Call 'uxtheme::SetWindowTheme(p r0, w "DarkMode_Explorer", p 0)'
    SetCtlColors $0 "${PV_TEXT}" "${PV_BG}"
    SendMessage $0 ${WM_SETFONT} $PvFontBody 1
    System::Call 'user32::SetWindowPos(p r0, p 0, i $PvX, i $PvY, i $PvW, i $PvH, i 0x40)'
  FunctionEnd
!macroend

!macro PV_HIDE ID
  GetDlgItem $0 $HWNDPARENT ${ID}
  System::Call 'user32::MoveWindow(p r0, i -3000, i 0, i 1, i 1, i 0)'
  ShowWindow $0 ${SW_HIDE}
!macroend

; ---------------------------------------------------------------- installer pages

!macroundef MUI_PAGE_WELCOME
!macro MUI_PAGE_WELCOME
  !insertmacro MUI_PAGE_INIT
  !insertmacro MUI_UNSET MUI_PAGE_CUSTOMFUNCTION_PRE
  !insertmacro PV_FUNCTIONS ""
  AutoCloseWindow true
  MiscButtonText "" "" "Close" ""
  Caption "$(^Name) Setup"
  SubCaption 3 " "
  SubCaption 4 " "
  UninstallSubCaption 1 " "
  UninstallSubCaption 2 " "

  Page custom PvWelcome PvWelcomeLeave
  Function PvWelcome
    ${If} $PassiveMode = 1
      Abort
    ${EndIf}
    ReadRegStr $PvOldVersion SHCTX "${UNINSTKEY}" "DisplayVersion"
    StrCpy $R8 "Install ${PRODUCTNAME}"
    StrCpy $R9 "Version ${VERSION}. For your account only, no administrator needed."
    StrCpy $R7 "Install"
    ${If} $PvOldVersion != ""
      nsis_tauri_utils::SemverCompare "${VERSION}" $PvOldVersion
      Pop $0
      StrCpy $R8 "Update ${PRODUCTNAME}"
      StrCpy $R9 "From $PvOldVersion to ${VERSION}. Your downloads and settings stay."
      StrCpy $R7 "Update"
      ${If} $0 = 0
        StrCpy $R8 "Reinstall ${PRODUCTNAME}"
        StrCpy $R9 "Version ${VERSION} is already installed. Your downloads and settings stay."
        StrCpy $R7 "Reinstall"
      ${ElseIf} $0 = -1
        StrCpy $R8 "Install an older version?"
        StrCpy $R9 "Version $PvOldVersion is installed; this is the older ${VERSION}."
        StrCpy $R7 "Install"
      ${EndIf}
    ${EndIf}
    Call PvBegin

    !insertmacro PV_LABEL 40 150 400 20 "Install in" $PvFontSmall "${PV_MUTED}" "${PV_BG}"
    Pop $0
    !insertmacro PV_RECT 52 183 360 20
    nsDialogs::CreateControl EDIT ${DEFAULT_STYLES}|${WS_TABSTOP}|${ES_AUTOHSCROLL} 0 $PvX $PvY $PvW $PvH "$INSTDIR"
    Pop $PvFolder
    SendMessage $PvFolder ${WM_SETFONT} $PvFontBody 1
    SetCtlColors $PvFolder "${PV_TEXT}" "${PV_FIELD}"
    !insertmacro PV_LABEL 40 174 384 36 "" $PvFontBody "${PV_TEXT}" "${PV_FIELD}"
    Pop $0
    ${Px} $1 20
    System::Call 'gdi32::CreateRoundRectRgn(i 0, i 0, i $PvW, i $PvH, i r1, i r1) p .r2'
    System::Call 'user32::SetWindowRgn(p r0, p r2, i 1)'
    !insertmacro PV_BUTTON 432 174 88 36 "Change" "" PvBrowse
    Pop $0

    !insertmacro PV_CHECKBOX 40 230 400 24 "Desktop shortcut" ""
    Pop $PvDesktopCheck
    ${If} $PvOldVersion == ""
    ${OrIf} ${FileExists} "$DESKTOP\${PRODUCTNAME}.lnk"
      ${NSD_Check} $PvDesktopCheck
    ${EndIf}

    !insertmacro PV_BUTTON 400 340 120 36 "$R7" "" PvNext
    Pop $PvPrimary
    !insertmacro PV_BUTTON 290 340 100 36 "Cancel" "" PvCancel
    Pop $0
    ${NSD_SetFocus} $PvPrimary
    Call PvShow
  FunctionEnd

  Function PvBrowse
    Pop $0
    ${NSD_GetText} $PvFolder $1
    nsDialogs::SelectFolderDialog "Install ${PRODUCTNAME} in" "$1"
    Pop $0
    ${If} $0 != "error"
      ${GetFileName} "$0" $1
      ${If} $1 != "${PRODUCTNAME}"
        StrCpy $0 "$0\${PRODUCTNAME}"
      ${EndIf}
      ${NSD_SetText} $PvFolder "$0"
    ${EndIf}
  FunctionEnd

  Function PvWelcomeLeave
    ${NSD_GetText} $PvFolder $0
    StrCpy $1 $0 2
    StrCpy $2 $0 1 1
    StrLen $3 $0
    ${If} $3 < 4
    ${OrIfNot} $1 == "\\"
    ${AndIfNot} $2 == ":"
      MessageBox MB_ICONEXCLAMATION "Choose a full folder path, such as $LOCALAPPDATA\${PRODUCTNAME}."
      Abort
    ${EndIf}
    StrCpy $INSTDIR $0
    ${NSD_GetState} $PvDesktopCheck $PvDesktop

    ; An MSI installation is a separate one (for all users): it goes first.
    Call FindWixInstall
    ${If} $R6 != ""
      MessageBox MB_OKCANCEL|MB_ICONINFORMATION "${PRODUCTNAME} was installed with the MSI package. That copy is removed first; Windows asks for permission. Your downloads and settings stay." IDOK remove_msi
      Abort
      remove_msi:
      ReadRegStr $R1 HKLM "$R6" "UninstallString"
      HideWindow
      ExecWait '$R1' $0
      BringToFront
      ${If} $0 <> 0
        Abort
      ${EndIf}
    ${EndIf}

    ; Tauri's next page offers to uninstall first. It is not needed: the new version goes
    ; over the old one, as the updater does. These modes make it skip itself.
    ${If} $PassiveMode <> 1
    ${AndIf} $UpdateMode <> 1
      StrCpy $PvSkipReinstall 1
      StrCpy $PassiveMode 1
      StrCpy $UpdateMode 1
    ${EndIf}
  FunctionEnd
!macroend

; In place of the folder page (the folder is on the first one): back to the real modes.
!macroundef MUI_PAGE_DIRECTORY
!macro MUI_PAGE_DIRECTORY
  !insertmacro MUI_UNSET MUI_PAGE_CUSTOMFUNCTION_PRE
  Page custom PvRestoreModes
  Function PvRestoreModes
    ${If} $PvSkipReinstall = 1
      StrCpy $PvSkipReinstall 0
      StrCpy $PassiveMode 0
      StrCpy $UpdateMode 0
    ${EndIf}
  FunctionEnd
!macroend

!macroundef MUI_PAGE_INSTFILES
!macro MUI_PAGE_INSTFILES
  !insertmacro MUI_PAGE_INIT
  PageEx instfiles
    Caption " "
    PageCallbacks "" PvInstallShow ""
  PageExEnd
  Function PvInstallShow
    StrCpy $R8 "Installing ${PRODUCTNAME} ${VERSION}"
    Call PvStyleProgress
  FunctionEnd
!macroend

!macroundef MUI_PAGE_FINISH
!macro MUI_PAGE_FINISH
  !insertmacro MUI_PAGE_INIT
  !insertmacro MUI_UNSET MUI_PAGE_CUSTOMFUNCTION_PRE
  Page custom PvFinish PvFinishLeave
  Function PvFinish
    ${If} $PassiveMode = 1
      Abort
    ${EndIf}
    ${If} $PvDesktop = 1
      Call CreateOrUpdateDesktopShortcut
    ${EndIf}
    StrCpy $R8 "${PRODUCTNAME} is ready"
    StrCpy $R9 "Version ${VERSION} is installed. New versions are offered in the app."
    Call PvBegin
    !insertmacro PV_CHECKBOX 40 174 400 24 "Start ${PRODUCTNAME} now" ""
    Pop $PvStartCheck
    ${NSD_Check} $PvStartCheck
    !insertmacro PV_BUTTON 400 340 120 36 "Finish" "" PvNext
    Pop $PvPrimary
    ${NSD_SetFocus} $PvPrimary
    Call PvShow
  FunctionEnd
  Function PvFinishLeave
    ${NSD_GetState} $PvStartCheck $0
    ${If} $0 = 1
      Call RunMainBinary
    ${EndIf}
  FunctionEnd
!macroend

; -------------------------------------------------------------- uninstaller pages

!macroundef MUI_UNPAGE_CONFIRM
!macro MUI_UNPAGE_CONFIRM
  !insertmacro MUI_UNPAGE_INIT
  !insertmacro MUI_UNSET MUI_PAGE_CUSTOMFUNCTION_PRE
  !insertmacro MUI_UNSET MUI_PAGE_CUSTOMFUNCTION_SHOW
  !insertmacro MUI_UNSET MUI_PAGE_CUSTOMFUNCTION_LEAVE
  !insertmacro PV_FUNCTIONS "un."

  UninstPage custom un.PvConfirm un.PvConfirmLeave
  Function un.PvConfirm
    ${If} $PassiveMode = 1
      Abort
    ${EndIf}
    StrCpy $R8 "Remove ${PRODUCTNAME}?"
    StrCpy $R9 "Files you downloaded are kept."
    Call un.PvBegin
    !insertmacro PV_CHECKBOX 40 174 480 24 "Also delete settings and the download list" "un."
    Pop $DeleteAppDataCheckbox
    !insertmacro PV_LABEL 68 202 440 40 "Leave this off to find everything again after a new installation." $PvFontSmall "${PV_MUTED}" "${PV_BG}"
    Pop $0
    !insertmacro PV_BUTTON 400 340 120 36 "Remove" "un." un.PvNext
    Pop $PvPrimary
    !insertmacro PV_BUTTON 290 340 100 36 "Cancel" "un." un.PvCancel
    Pop $0
    ${NSD_SetFocus} $0
    Call un.PvShow
  FunctionEnd
  Function un.PvConfirmLeave
    ${NSD_GetState} $DeleteAppDataCheckbox $DeleteAppDataCheckboxState
  FunctionEnd
!macroend

!macroundef MUI_UNPAGE_INSTFILES
!macro MUI_UNPAGE_INSTFILES
  !insertmacro MUI_UNPAGE_INIT
  PageEx un.instfiles
    Caption " "
    PageCallbacks "" un.PvRemoveShow ""
  PageExEnd
  Function un.PvRemoveShow
    StrCpy $R8 "Removing ${PRODUCTNAME}"
    Call un.PvStyleProgress
  FunctionEnd

  UninstPage custom un.PvRemoved
  Function un.PvRemoved
    ${If} $PassiveMode = 1
    ${OrIf} $UpdateMode = 1
      Abort
    ${EndIf}
    StrCpy $R8 "${PRODUCTNAME} was removed"
    StrCpy $R9 "Thank you for using it."
    Call un.PvBegin
    !insertmacro PV_BUTTON 400 340 120 36 "Close" "un." un.PvNext
    Pop $PvPrimary
    ${NSD_SetFocus} $PvPrimary
    Call un.PvShow
  FunctionEnd
!macroend

; -------------------------------------------------------------------------- hooks

; Windows keeps showing a program's old icon in the taskbar, because it remembers the one
; of the Start menu shortcut and an update does not touch that. Saving it again (with its
; icon named explicitly) makes Windows look.
!macro PV_REFRESH_SHORTCUT LNK
  ${If} ${FileExists} "${LNK}"
    !insertmacro IsShortcutTarget "${LNK}" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 = 1
      CreateShortcut "${LNK}" "$INSTDIR\${MAINBINARYNAME}.exe" "" "$INSTDIR\${MAINBINARYNAME}.exe" 0
      !insertmacro SetLnkAppUserModelId "${LNK}"
      System::Call 'shell32::SHChangeNotify(i 0x2000, i 0x1005, w "${LNK}", p 0)'
    ${EndIf}
  ${EndIf}
!macroend

; On to the last page.
!macro NSIS_HOOK_POSTUNINSTALL
  SetAutoClose true
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro PV_REFRESH_SHORTCUT "$SMPROGRAMS\${PRODUCTNAME}.lnk"
  !insertmacro PV_REFRESH_SHORTCUT "$DESKTOP\${PRODUCTNAME}.lnk"
!macroend
