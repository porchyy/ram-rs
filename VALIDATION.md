# ผลตรวจสอบ RAM Observer

## อัปเดต: โหมดเบื้องหลังตามคำขอผู้ใช้

หลังผู้ใช้ยืนยันให้รันเงียบ ๆ และลดภาระ เพิ่มโหมด `--background` เป็นค่าเริ่มต้น ขอบเขตการทดลองขยายเป็นแอปผู้ใช้เดียวกันที่ทุกหน้าต่างย่อไว้ ใช้ CPU ไม่เกิน 1% ของหนึ่ง core จากตัวอย่างต่อเนื่องอย่างน้อย 60 วินาที และ working set ≥128 MiB โดยยังไม่ถือว่าผ่านการพิสูจน์ประโยชน์ด้านความเร็วของการ trim

- `fmt --check`, `cargo test` 5 tests, `cargo clippy --all-targets -- -D warnings` และ release build ผ่าน
- self-check ตรวจ token SID, critical/protection APIs, การยกเว้นตัวเอง และ EnumWindows แบบอ่านอย่างเดียว ผ่านร่วมกับการทดลองโปรเซสลูกเดิม
- ทดสอบ release เปิด `--background`, เปิดซ้ำแล้วตัวที่สองออกด้วยรหัส 0, `--status` อ่านสถานะได้, `--stop` ทำให้ตัวจัดการออกอย่างสุภาพด้วยรหัส 0 แล้วเปิดตัวจริงใหม่สำเร็จ
- ยืนยัน MainWindowHandle = 0, priority = BelowNormal ไม่ได้เปิด UI หรือสร้างโปรเซสทดสอบในโหมด background
- ตัวอย่างเริ่มต้น working set 5.35 MiB, private memory 1.66 MiB; หลังช่วงรอ 30 วินาที working set 5.78 MiB, private memory 1.57 MiB CPU time เพิ่มจาก 0.015625 เป็น 0.15625 วินาทีระหว่างการตรวจ ข้อมูลดิบเก็บเฉพาะในเครื่องทดสอบ ไม่รวมไฟล์ `dist/` หรือ log ใน repository
- ในช่วงเริ่มต้น RAM ประมาณ 86–87% จึงยังไม่มีคำขอ trim โปรเซสภายนอก ไม่สร้างแรงกดดัน RAM ปลอมเพื่อบังคับทดสอบ และยังไม่ได้ยืนยันประโยชน์ของโหมดนี้กับ workload จริงระยะยาว

ใช้ RAM/commit sample ทุก 2 วินาที; เริ่มสำรวจเฉพาะหน้าต่างทุก 10 วินาทีเมื่อ RAM ≥85%; trim ได้เมื่อมีเป้าหมายพร้อมและ RAM ≥90% ต่อเนื่อง 10 วินาที จำกัด 2 โปรเซสต่อรอบ พัก 120 วินาทีและ rearm เมื่อ RAM <85% ต่อเนื่อง 10 วินาทีหลังพ้นพัก Handle เดิมต้องยังมีชีวิต SID ต้องตรงกัน โปรเซส critical/protected, Windows directory, ตัวเอง และ foreground ถูกข้าม หากตรวจข้อมูลไม่ได้ไม่สั่ง trim

เงื่อนไขหน้าต่างย่อและ CPU ต่ำเป็น heuristic ซึ่งยังอาจกระทบงานเบื้องหลังหรือทำให้กลับมาใช้แอปช้าลง ไม่มีการเปลี่ยน startup, registry, page file หรือ memory compression ไม่มีการอ้างว่าเป็นตัวจัดการที่ดีที่สุด

## รายงานรุ่นหน้าต่างก่อนเพิ่มโหมดเบื้องหลัง (ประวัติ)

วันที่ทดสอบ: 18 พฤศจิกายน 2026 — Windows x64 (ระบบรายงาน Windows 10 Pro), Rust stable GNU ที่ติดตั้งอยู่เดิม ไม่ติดตั้ง toolchain หรือปรับ Windows

## สิ่งที่ผ่าน

- `cargo test`: 3 tests ผ่าน ตรวจ threshold ที่เท่ากับ 90%, สูงชั่วครู่/สูงต่อเนื่อง, cooldown, ขอบเขตต่ำกว่า 85%, rearm, ปิด/เปิดสวิตช์, ข้อมูลหาย/NaN, ช่องว่าง sampling และการจำกัด/หมุน log
- `cargo clippy --all-targets -- -D warnings` และ formatter check ผ่าน
- Debug build และ release build สำเร็จ สคริปต์ `build.ps1` ใช้ target directory ชื่ออังกฤษเพื่อเลี่ยงปัญหา GNU dlltool กับพาธภาษาไทย
- `--self-check`: cancellation ข้ามการ trim, handle ที่ไม่มี PROCESS_SET_QUOTA ถูกปฏิเสธ และการเริ่มรอบกับโปรเซสลูกที่ออกแล้วถูกปฏิเสธ
- handle เดิมจาก Child ใช้ตลอดการทดลอง ไม่เปิด PID ใหม่เพื่อสั่ง trim ไม่มีรายการอนุญาตของโปรเซสภายนอก

## ตรวจค่าระบบ

ตัวอย่าง `--snapshot`: total physical 7,896,100,864 bytes ตรงกับ CIM TotalVisibleMemorySize 7,711,036 KiB × 1,024 ค่าพร้อมใช้ต่างกันได้ตามเวลาอ่าน โดย snapshot ช่วงหนึ่งแสดง 674,340,864 bytes และ CIM ที่อ่านภายหลังแสดง 664,176 KiB ไม่ใช่การอ่านพร้อมกัน

Commit snapshot: 17,668,096,000 / 25,075,970,048 bytes ตัวเลขนี้แสดงแยกจาก physical RAM ไม่ตีความว่าใช้ RAM จริงเกินขนาดที่ติดตั้ง ดู [Microsoft Memory Performance Information](https://learn.microsoft.com/en-us/windows/win32/memory/memory-performance-information)

## การทดลองควบคุม

ข้อมูลจาก `--self-check` บนโปรเซสลูกที่จัดสรร 64 MiB:

| ตัววัด | รอบ 1 | รอบ 2 | รอบ 3 |
|---|---:|---:|---:|
| Working set ก่อน | 68.4 MiB | 68.4 MiB | 68.4 MiB |
| หลัง EmptyWorkingSet | ประมาณ 0.0 MiB | ประมาณ 0.0 MiB | ประมาณ 0.0 MiB |
| หลังกลับมาแตะข้อมูล | 64.4 MiB | 64.4 MiB | 64.4 MiB |
| เวลาแตะก่อน trim | 1,711 µs | 1,182 µs | 1,408 µs |
| เวลาแตะหลัง trim | 42,959 µs | 36,479 µs | 35,825 µs |
| Page faults ตอนกลับมาใช้ | 16,497 | 16,496 | 16,499 |

สถิติเหล่านี้มาจาก debug build และ workload สังเคราะห์ ไม่ใช่ benchmark ของแอปจริง PageFaultCount รวม soft faults และ read I/O รวมการอ่าน pipe จึงยังแยก hard faults/เวลาใช้งานดิสก์ไม่ได้

ผลคือการลด working set เรียก API ได้ แต่ข้อมูลกลับมาอยู่ใน working set เมื่อใช้งาน และการแตะข้อมูลครั้งถัดไปช้าลงอย่างชัดเจน ไม่แสดงหลักฐานว่าผู้ใช้จะทำงานลื่นขึ้น ข้อค้นพบนี้สอดคล้องกับความหมายของ [EmptyWorkingSet](https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-emptyworkingset) ซึ่งเอา pages ออกจาก working set ไม่ได้คืน allocation

**เกณฑ์ข้อ 3 ของ PLAN.md ยังไม่ผ่าน จึงไม่เปิดการคืนพื้นที่อัตโนมัติกับโปรเซสภายนอก** Auto ในโปรแกรมเป็นการทดสอบ policy กับโปรเซสลูกของตัวเองเท่านั้น ก่อนขยายขอบเขตต้องทบทวนแนวทางกับผู้ใช้และวัด workload จริง รวมกิจกรรม paging/ดิสก์และเวลาโต้ตอบ

หมายเหตุการทดสอบ: GetProcessMemoryInfo อาจยังสำเร็จกับ handle ของโปรเซสที่ออกแล้ว ดังนั้นตรวจการปฏิเสธผ่านเส้นทาง `experiment` ที่เช็ก Child::try_wait ก่อนทำงาน ไม่สมมติว่าการอ่านสถิติต้องล้มเหลว

## สิ่งที่ยังตรวจไม่ได้

Computer Use ทั้ง node_repl และ cua_repl เริ่มไม่ได้ โดยแจ้ง `windows sandbox failed: helper_unknown_error: setup refresh had errors` จึงไม่ได้ยืนยันภาพหน้าต่าง การคลิกจริง keyboard navigation, screen reader, DPI scaling และการปิดหน้าต่างผ่าน UI ไม่อ้างว่าการตรวจ UI ผ่านจาก build อย่างเดียว

ยังไม่ได้ตรวจ Windows หลายเครื่อง, CPU/RAM ของตัวจัดการระยะยาว, สลับแอปจริง, standby/page-file behavior หรือประโยชน์ของการ trim แอปภายนอก ไม่มีการเปลี่ยน startup, registry, page file หรือการตั้งค่าระบบเพื่อทดสอบ
