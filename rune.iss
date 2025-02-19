; Script generated by the Inno Setup Script Wizard.
; SEE THE DOCUMENTATION FOR DETAILS ON CREATING INNO SETUP SCRIPT FILES!

#define MyAppName "Rune"
#define MyAppVersion "1.0"
#define MyAppPublisher "Rune Developers"
#define MyAppURL "https://rune.not.ci"
#define MyAppExeName "rune.exe"

[Setup]
; NOTE: The value of AppId uniquely identifies this application. Do not use the same AppId value in installers for other applications.
; (To generate a new GUID, click Tools | Generate GUID inside the IDE.)
AppId={{74D23F28-AAA0-4FF5-BAF9-619B8A8DF6D9}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
;AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppName}
; "ArchitecturesAllowed=x64compatible" specifies that Setup cannot run
; on anything but x64 and Windows 11 on Arm.
ArchitecturesAllowed=x64compatible
; "ArchitecturesInstallIn64BitMode=x64compatible" requests that the
; install be done in "64-bit mode" on x64 or Windows 11 on Arm,
; meaning it should use the native 64-bit Program Files directory and
; the 64-bit view of the registry.
ArchitecturesInstallIn64BitMode=x64compatible
DisableProgramGroupPage=yes
LicenseFile=.\LICENSE
; Remove the following line to run in administrative install mode (install for all users.)
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=commandline
OutputBaseFilename=Rune
Compression=lzma
SolidCompression=yes
WizardStyle=modern
CloseApplications=force

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: ".\build\windows\x64\runner\Release\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs
; NOTE: Don't use "Flags: ignoreversion" on any shared system files

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Code]
function GetCommandlineParam(ParamName: String): Boolean;
var
  I: Integer;
  Param: String;
begin
  Result := False;
  for I := 1 to ParamCount do
  begin
    Param := ParamStr(I);
    if (Param = '--' + ParamName) or (Param = '/' + ParamName) then
    begin
      Result := True;
      Break;
    end;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  AppDir: String;
  ProFile: String;
begin
  if CurStep = ssInstall then
  begin
    if GetCommandlineParam('pro') then
    begin
      AppDir := ExpandConstant('{app}');
      ProFile := AppDir + '\.pro';
      if not ForceDirectories(AppDir) then
      begin
        MsgBox('Failed to create app directory', mbError, MB_OK);
        Exit;
      end;
      
      if not SaveStringToFile(ProFile, '', False) then
      begin
        MsgBox('Failed to create license file', mbError, MB_OK);
      end;
    end;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  AppDir: string;
  ProFile: string;
  ResultCode: Integer;
begin
  if CurUninstallStep = usUninstall then
  begin
    AppDir := ExpandConstant('{app}');
    ProFile := AppDir + '\.pro';
    
    if FileExists(ProFile) then
    begin
      if not DeleteFile(ProFile) then
      begin
        MsgBox('Unable to delete the license file.', mbError, MB_OK);
      end;
    end;
  end;

  if CurUninstallStep = usDone then
  begin
    Exec('cmd.exe /c "rmdir /S /Q' + appDir + '"', '', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  end;
end;

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent

