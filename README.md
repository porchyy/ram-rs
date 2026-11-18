# RAM Observer

โปรแกรม Rust สำหรับ Windows 10/11 x64 เริ่มต้นทำงานเบื้องหลังเงียบ ๆ ไม่มี UI ใช้ `windows-sys` และ standard library ไม่มี web runtime

โปรเจ็กต์นี้ติดตามแรงกดดันของ RAM และทดลองขอให้ Windows ลด working set ของแอปที่ย่อหน้าต่างและใช้ CPU ต่ำเมื่อ RAM ใกล้เต็ม เป้าหมายคือทำงานโดยมีภาระต่ำ ไม่ปิดแอป และไม่แทรกแซงแอป foreground

**สถานะ: โหมดคืน working set แบบทดลองตามคำขอให้ทำงานเบื้องหลัง** ผลทดลองเดิมพบว่ากลับมาใช้ข้อมูลช้าลง จึงจำกัดเป้าหมายเป็นแอปที่ย่อหน้าต่างและมี CPU ต่ำ ไม่อ้างว่าดีกว่าการจัดการหน่วยความจำของ Windows อ่านผลและข้อจำกัดที่ [VALIDATION.md](VALIDATION.md)

## เริ่มจากซอร์สโค้ด

ต้องใช้ Windows, Rust stable และ linker ที่เข้ากับ toolchain (MSVC Build Tools หรือ MinGW สำหรับ GNU) โปรเจ็กต์ไม่ติดตั้งหรือเปลี่ยนการตั้งค่าเหล่านี้ให้

```powershell
git clone https://github.com/porchyy/ram-rs.git
cd ram-rs
.\build.ps1
Start-Process -FilePath .\dist\ram-observer.exe -WindowStyle Hidden
```

โปรแกรมทำงานต่อจนสั่งหยุดหรือออกจาก Windows และไม่ตั้งเปิดพร้อมเครื่อง หากรันซ้ำจะมีตัวจัดการเพียงหนึ่งตัวต่อ Windows session ไฟล์ใน `dist/` สร้างจาก build ไม่ได้เก็บไว้ใน Git

## ทำงานอย่างไร

1. อ่าน physical RAM และ commit จาก `GlobalMemoryStatusEx` / `GetPerformanceInfo` ทุก 2 วินาที
2. เมื่อ RAM ≥85% สำรวจหน้าต่างเพื่อหาโปรเซสที่ทุกหน้าต่างย่ออยู่ ติดตามเวลา CPU ด้วย `GetProcessTimes` และรอให้เข้าเงื่อนไขอย่างน้อย 60 วินาที
3. เมื่อ RAM ≥90% ต่อเนื่อง 10 วินาทีและมีเป้าหมายพร้อม ตรวจ SID, path, สถานะ critical/protected, foreground และ handle ที่ยังมีชีวิตอีกครั้ง
4. เรียก `EmptyWorkingSet` ได้ไม่เกิน 2 โปรเซสต่อรอบ การเรียกนี้เอา pages ออกจาก working set ไม่ได้คืน allocation ของแอปหรือเพิ่ม RAM จริง
5. วัด working set และ RAM ที่พร้อมใช้หลังรอบ บันทึกผล แล้วพักอย่างน้อย 120 วินาที ต้องเห็น RAM <85% ต่อเนื่อง 10 วินาทีหลังพักก่อนพร้อมรอบใหม่ หาก RAM สูงค้างจะไม่ล้างวน

ตัวอย่างภาระของตัวจัดการในเครื่องทดสอบ: working set ประมาณ 5–6 MiB และใช้ priority `BelowNormal` ตัวเลขนี้เป็นการวัดระยะสั้นจากเครื่องเดียว ไม่ใช่การรับประกันสำหรับทุกเครื่อง

## เปิดใช้งาน

ดับเบิลคลิก `dist\ram-observer.exe` หลัง build ไม่มี installer ไม่ต้องใช้ administrator

- อ่าน RAM/commit ทุก 2 วินาที ใช้ priority ต่ำกว่าปกติ และไม่มีโปรเซสทดสอบ 64 MiB ในโหมดนี้
- เมื่อ RAM ตั้งแต่ 85% จึงตรวจรายการหน้าต่างทุก 10 วินาที ไม่สแกนรายชื่อโปรเซสทั้งระบบตามรอบ
- เป้าหมายต้องเป็นโปรเซสผู้ใช้เดียวกันที่ทุกหน้าต่างที่มองเห็นถูกย่อไว้ จากตัวอย่างที่สังเกตนานอย่างน้อย 60 วินาที และใช้ CPU ไม่เกิน 1% ของหนึ่ง core ระหว่างตัวอย่าง ต้องมี working set อย่างน้อย 128 MiB
- เมื่อมีเป้าหมายพร้อมและ RAM ≥90% ต่อเนื่อง 10 วินาที จึงทดลอง trim ไม่เกิน 2 โปรเซสต่อรอบ พัก 120 วินาที และต้องต่ำกว่า 85% ต่อเนื่อง 10 วินาทีหลังช่วงพักก่อนพร้อมรอบใหม่ หาก RAM สูงค้างจะไม่ล้างวน
- ตรวจหน้าต่าง foreground, สถานะย่อ, SID, Windows-directory/ตัวจัดการเอง, critical/protected process และ handle เดิมก่อนดำเนินการ หากตรวจไม่ได้ให้ข้าม ไม่มีการบังคับปิดโปรเซส
- หน้าต่างย่อและ CPU ต่ำเป็นเพียงตัวประมาณความไม่ใช้งาน แอปที่ทำงานเบื้องหลังอยู่ก็อาจได้รับผลและช้าลงตอนเรียกกลับ ไม่ใช่ผลพิสูจน์ว่าทำให้เครื่องเร็วขึ้น
- เก็บสถานะทุกประมาณ 60 วินาทีใน `%LOCALAPPDATA%\RamObserver\background-status.txt` และ log เฉพาะเริ่ม/หยุด/รอบทดลอง ไม่เขียน log ทุก sample เปิดซ้ำแล้วไม่สร้างตัวจัดการซ้อน
- ปิดโปรแกรมแล้วไม่เหลือการตั้งค่าระบบ ไม่มีการตั้ง startup อัตโนมัติ

คำสั่งควบคุม (ไม่มีหน้าต่าง):

```powershell
.\dist\ram-observer.exe --background
.\dist\ram-observer.exe --stop
```

`--stop` ส่งสัญญาณหยุดอย่างสุภาพ ตรวจภายในรอบรอ 2 วินาที และก่อนรอบถัดไป API ที่เริ่มไปแล้วไม่สามารถย้อนกลับได้ ดูสถานะผ่านไฟล์ด้านบน หรือ `cargo run -- --status` ใน debug console

| คำสั่ง | หน้าที่ |
|---|---|
| ไม่มี argument หรือ `--background` | เริ่มตัวจัดการเงียบ ๆ |
| `--stop` | หยุดตัวจัดการเบื้องหลัง |
| `--status` | อ่านสถานะตัวจัดการที่กำลังรัน |
| `--snapshot` | อ่าน RAM/commit และจำนวนโปรเซสครั้งเดียว ไม่ trim |
| `--self-check` | ทดสอบ API/ตัวกรองและ trim เฉพาะโปรเซสลูกทดสอบ |
| `--ui` | เปิดหน้าต่างดูข้อมูลและห้องทดลองแบบควบคุม |

ใช้ debug build หรือ `cargo run -- <คำสั่ง>` เมื่อต้องการอ่านข้อความใน console เพราะ release เป็น Windows GUI-subsystem executable เพื่อไม่ให้เกิดหน้าต่าง console ระหว่างรันเงียบ

## หน้าต่างทดลอง (ไม่เปิดตามปกติ)

เปิดด้วย `dist\ram-observer.exe --ui` เฉพาะเมื่ออยากดูข้อมูล/ทดสอบด้วยตนเอง

- หน้าดูข้อมูลทำงานทันที อัปเดต RAM ประมาณทุกวินาที และรายการโปรเซสทุก 5 วินาที
- **Run controlled test (64 MiB)** สร้างโปรเซสลูกที่จัดสรรข้อมูล 64 MiB แล้วเปรียบเทียบ working set ก่อน/หลัง EmptyWorkingSet และหลังแตะข้อมูลอีกครั้ง พร้อมเวลาและ page faults
- **Auto (test process only)** เริ่มต้น OFF เปิดได้เฉพาะเพื่อทดสอบตรรกะกับโปรเซสลูกเดิม: RAM ≥90% ต่อเนื่อง 10 วินาที พัก 120 วินาที และต้องลดต่ำกว่า 85% ต่อเนื่อง 10 วินาทีหลังพ้นช่วงพักก่อนพร้อมรอบใหม่ ไม่มีเป้าหมายจนกว่าจะรัน controlled test
- ปิด Auto เพื่อหยุดรอบถัดไป; ตรวจสวิตช์อีกครั้งก่อนเรียก API การเรียก API ที่เริ่มไปแล้วไม่สามารถย้อนกลับได้ ปิดหน้าต่างเพื่อออก โปรเซสทดสอบออกเมื่อ pipe ของโปรแกรมปิด
- ใช้ Tab / Shift+Tab สลับ control, Space กดปุ่ม, Alt+T รันทดสอบ และ Alt+A สลับ Auto ข้อมูลเป็นช่องอ่านอย่างเดียวที่เลือกและคัดลอกได้

โปรเซสลูกในหน้าต่างทดลองใช้ RAM ประมาณ 64 MiB จนปิดหน้าต่าง โหมดนี้ยังจำกัดการ trim ไว้เฉพาะโปรเซสทดสอบ ไม่ควรเปิด `--ui` หากต้องการตัวจัดการเบื้องหลังที่เบา

## Build

ต้องมี Rust stable และ Windows linker ที่ใช้กับ toolchain นั้นได้ สคริปต์ไม่ติดตั้งหรือเปลี่ยนค่าเริ่มต้นของ toolchain

```powershell
.\build.ps1
```

ได้ `dist\ram-observer.exe` สคริปต์ใช้ `%TEMP%\ram-observer-build` เป็นโฟลเดอร์ build เพราะ GNU dlltool ในเครื่องทดสอบอ่านพาธภาษาไทยไม่ได้ `Cargo.lock` ล็อก dependency แล้ว

## ตรวจสอบด้วยคำสั่ง

ใช้ debug build สำหรับข้อความใน console:

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
$env:CARGO_TARGET_DIR = Join-Path $env:TEMP 'ram-observer-target'
cargo test
cargo clippy --all-targets -- -D warnings
cargo run -- --snapshot
cargo run -- --self-check
cargo fmt --check
```

`--snapshot` อ่านอย่างเดียว ส่วน `--self-check` แตะเฉพาะโปรเซสทดสอบที่สร้างขึ้นใหม่ ตรวจ cancellation, handle ที่ไม่มีสิทธิ์ และโปรเซสที่ออกแล้ว ไม่เคยรับ PID จากผู้ใช้

เครื่องทดสอบมี rustfmt เฉพาะ MSVC toolchain จึงใช้ `cargo +stable-x86_64-pc-windows-msvc fmt --check` แทนคำสั่งสุดท้าย โดยยัง build ด้วย GNU toolchain เดิม

## ข้อมูลและข้อจำกัด

- บันทึกผลรอบใน `%LOCALAPPDATA%\RamObserver\events.log` จำกัด 1 MiB และ `events.previous.log` อีก 1 MiB; หากเขียนไม่ได้ UI แสดง LOG ERROR ส่วนโหมด background หยุดพร้อม error ทาง stderr ไม่เก็บเนื้อหาหน่วยความจำหรือส่งข้อมูลออกเครือข่าย
- RAM ใช้งาน = `(total physical - available physical) / total physical`; commit มาจาก GetPerformanceInfo และแสดงแยก ไม่ใช้แทน RAM จริง
- working set รวมหน้าที่แชร์กับโปรเซสอื่นได้ จึงห้ามรวมแต่ละแถวเป็น RAM ทั้งเครื่อง โปรเซสที่อ่านไม่ได้หรือออกระหว่างอ่านถูกข้ามพร้อมจำนวนที่แสดง
- การคืน working set ไม่ได้คืน allocation หรือแก้ memory leak และอาจทำให้เกิด page faults มากขึ้น ดู [Microsoft EmptyWorkingSet](https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-emptyworkingset) และ [Working Set](https://learn.microsoft.com/en-us/windows/win32/memory/working-set)
- read I/O ในผลทดลองเป็น process I/O ซึ่งรวม pipe ไม่ใช่เครื่องวัด hard faults หรือ latency ของดิสก์ ยังไม่ได้ทดสอบ workload จริงระยะยาว
- ไม่มี startup, tray, service, เปลี่ยน registry/page file, ล้าง standby cache, ปิด memory compression, suspend หรือบังคับจบโปรเซสอื่น
- การตรวจภาพหน้าต่าง ปุ่มจริง screen reader และหลาย DPI ยังไม่ผ่านการตรวจด้วยเครื่องมือ เนื่องจาก Computer Use เริ่มไม่ได้ใน session นี้

## โครงสร้าง

- `src/system.rs`: ตัวอ่าน Windows API และ handle ownership
- `src/background.rs`: โหมดเงียบ, ตัวกรองแอปที่ย่อไว้, singleton และสัญญาณหยุด
- `src/policy.rs`: เกณฑ์ต่อเนื่อง, cooldown และ rearm พร้อมข้อมูลจำลอง
- `src/lab.rs`: โปรเซสทดสอบและการวัดผล ใช้ handle เดิมตลอดเพื่อป้องกัน PID reuse
- `src/app.rs`: worker หนึ่งตัว, snapshot, สถานะ และ log แบบจำกัดขนาด
- `src/ui.rs`: หน้าต่าง native ที่อ่านข้อมูลจาก worker โดยไม่รอการทดลองบน UI thread
