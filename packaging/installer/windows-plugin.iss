[Code]
var
  XPlanePage: TInputDirWizardPage;
  Detected: TNewComboBox;

function Quoted(Value: String): String;
begin
  Result := '"' + Value + '"';
end;

function HelperError: String;
var Lines: TArrayOfString; I: Integer;
begin
  Result := '';
  if LoadStringsFromFile(ExpandConstant('{tmp}\openatc-error.txt'), Lines) then
    for I := 0 to GetArrayLength(Lines) - 1 do Result := Result + Lines[I] + #13#10;
  if Result = '' then Result := 'OpenATC could not complete this operation. Check the selected folder, close X-Plane, and retry.';
end;

function RunHelper(Filename, Params: String): Boolean;
var Code: Integer;
begin
  DeleteFile(ExpandConstant('{tmp}\openatc-error.txt'));
  Result := Exec(Filename, Params + ' --error-file ' + Quoted(ExpandConstant('{tmp}\openatc-error.txt')),
    '', SW_HIDE, ewWaitUntilTerminated, Code);
  if Result then Result := Code = 0;
end;

procedure SelectDetected(Sender: TObject);
begin
  if Detected.ItemIndex >= 0 then XPlanePage.Values[0] := Detected.Items[Detected.ItemIndex];
end;

procedure InitializeWizard;
var Paths: TArrayOfString; I: Integer; Initial: String;
begin
  ExtractTemporaryFile('openatc-install.exe');
  XPlanePage := CreateInputDirPage(wpSelectDir, 'Choose X-Plane 12',
    'Select the simulator installation to update.',
    'Choose a detected installation below, or browse to the X-Plane root folder. Select the X-Plane root, not Resources\plugins. Close X-Plane before installing.', False, '');
  XPlanePage.Add('X-Plane 12 root folder:');
  Detected := TNewComboBox.Create(XPlanePage);
  Detected.Parent := XPlanePage.Surface;
  Detected.Left := 0;
  Detected.Top := XPlanePage.Edits[0].Top + ScaleY(62);
  Detected.Width := XPlanePage.SurfaceWidth;
  Detected.Style := csDropDownList;
  Detected.OnChange := @SelectDetected;
  if RunHelper(ExpandConstant('{tmp}\openatc-install.exe'), '--discover-list ' + Quoted(ExpandConstant('{tmp}\xplane-paths.txt'))) then
    if LoadStringsFromFile(ExpandConstant('{tmp}\xplane-paths.txt'), Paths) then
      for I := 0 to GetArrayLength(Paths) - 1 do
        if Trim(Paths[I]) <> '' then Detected.Items.Add(Trim(Paths[I]));
  Initial := ExpandConstant('{param:XPLANEROOT|}');
  if Initial = '' then Initial := GetPreviousData('XPlaneRoot', '');
  if (Initial = '') and (Detected.Items.Count > 0) then Initial := Detected.Items[0];
  XPlanePage.Values[0] := Initial;
  Detected.Visible := Detected.Items.Count > 0;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if not RunHelper(ExpandConstant('{tmp}\openatc-install.exe'), '--check ' + Quoted(XPlanePage.Values[0])) then
    Result := HelperError;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    if not RunHelper(ExpandConstant('{app}\openatc-install.exe'),
      '--install ' + Quoted(XPlanePage.Values[0]) + ' --yes --record ' + Quoted(ExpandConstant('{app}\xplane-path.txt'))) then
      RaiseException(HelperError);
end;

procedure RegisterPreviousData(PreviousDataKey: Integer);
begin
  SetPreviousData(PreviousDataKey, 'XPlaneRoot', XPlanePage.Values[0]);
end;

function InitializeUninstall: Boolean;
begin
  Result := RunHelper(ExpandConstant('{app}\openatc-install.exe'), '--check-closed');
  if not Result then MsgBox(HelperError, mbError, MB_OK);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    if not RunHelper(ExpandConstant('{app}\openatc-install.exe'), '--uninstall-record ' + Quoted(ExpandConstant('{app}\xplane-path.txt')) + ' --yes') then
      RaiseException(HelperError);
end;
