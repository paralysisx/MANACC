import os, glob, winreg, datetime

# Check registry for LAV specifically
def check_reg(hive, path):
    try:
        key = winreg.OpenKey(hive, path)
        i = 0
        while True:
            try:
                subkey_name = winreg.EnumKey(key, i)
                subkey = winreg.OpenKey(key, subkey_name)
                try:
                    name = winreg.QueryValueEx(subkey, "DisplayName")[0]
                    if "LAV" in name:
                        loc = ""
                        try: loc = winreg.QueryValueEx(subkey, "InstallLocation")[0]
                        except: pass
                        print(f"Registry LAV: {name} -> '{loc}'")
                except: pass
                i += 1
            except OSError:
                break
    except Exception as e:
        pass

check_reg(winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall")
check_reg(winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall")
check_reg(winreg.HKEY_CURRENT_USER,  r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall")

# Search Program Files for LAV
for d in [r"C:\Program Files", r"C:\Program Files (x86)"]:
    for entry in os.listdir(d):
        if entry == "LAV":
            full = os.path.join(d, entry)
            print(f"\nFound LAV dir: {full}")
            for f in os.listdir(full):
                fp = os.path.join(full, f)
                mtime = datetime.datetime.fromtimestamp(os.path.getmtime(fp))
                print(f"  {f} - {mtime}")
