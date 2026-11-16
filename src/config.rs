use std::path::PathBuf;

pub const CLI_PATH: &str = r"C:\Nexon\MabinogiMobile\MabinogiMobile_CLI.exe";
pub const DEFAULT_COLLECT_INTERVAL_SECS: u64 = 600; // 기본 10분 (600초)

/// 5대 시세 비교 그룹 정의
pub const GROUP_HIGH_VALUE: &[&str] = &[
    "마력 깃든 용비늘",
    "그랜드 앙상블 토탈 패키지",
    "이상한 고양이의 특별 무기 선택 상자",
    "바람개비 모자 상자(거래가능)",
];

pub const GROUP_MAGIC_STONES: &[&str] = &[
    "허상의 마력석",
    "포식의 마력석",
    "심해의 마력석",
];

pub const GROUP_SOUL_STONES: &[&str] = &[
    "공명의 영혼석",
    "파동의 영혼석",
    "망령의 영혼석",
    "원념의 영혼석",
];

pub const GROUP_BOXES: &[&str] = &[
    "그랜드 앙상블 토탈 패키지",
    "이상한 고양이의 특별 무기 선택 상자",
    "바람개비 모자 상자(거래가능)",
];

pub const GROUP_BAGS: &[&str] = &[
    "조이풀 크루 컬렉션 백",
    "서머 바캉스 컬렉션 백",
    "카마인 에버모어 로즈 패치 컬렉션 백",
    "코지데이 컬렉션 백",
];

/// 기본 모니터링 대상 주요 품목 리스트 (잔영, 해연 주요 장비 기본 포함)
pub const DEFAULT_MONITORED_NAMES: &[&str] = &[
    "마력 깃든 용비늘",
    "허상의 마력석",
    "포식의 마력석",
    "심해의 마력석",
    "공명의 영혼석",
    "파동의 영혼석",
    "망령의 영혼석",
    "원념의 영혼석",
    "그랜드 앙상블 토탈 패키지",
    "조이풀 크루 컬렉션 백",
    "이상한 고양이의 특별 무기 선택 상자",
    "바람개비 모자 상자(거래가능)",
    "서머 바캉스 컬렉션 백",
    "카마인 에버모어 로즈 패치 컬렉션 백",
    "코지데이 컬렉션 백",
    // 잔영 대표 장비 기본 조사 항목
    "잔영의 라이트 크로스보우ZZ",
    "잔영의 가죽 갑옷 상의ZZ",
    "잔영의 가죽 갑옷 하의ZZ",
    "잔영의 페리도트 네크리스ZZ",
    "잔영의 페리도트 링ZZ",
    "잔영의 론 엣지소드ZZ",
    "잔영의 비늘 갑옷 상의ZZ",
    "잔영의 마블 힐링 완드ZZ",
    "잔영의 전투복 상의ZZ",
    // 해연 대표 장비 기본 조사 항목
    "해연의 라이트 크로스보우ZZ",
    "해연의 가죽 갑옷 상의ZZ",
    "해연의 가죽 갑옷 하의ZZ",
    "해연의 페리도트 네크리스ZZ",
    "해연의 페리도트 링ZZ",
    "해연의 론 엣지소드ZZ",
    "해연의 비늘 갑옷 상의ZZ",
    "해연의 마블 힐링 완드ZZ",
    "해연의 전투복 상의ZZ",
];

/// 장비 부위(슬롯) 구분
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EquipmentSlot {
    Weapon,
    Armor,
    Accessory,
    Other,
}

#[allow(dead_code)]
impl EquipmentSlot {
    pub fn label(&self) -> &'static str {
        match self {
            EquipmentSlot::Weapon => "무기",
            EquipmentSlot::Armor => "방어구",
            EquipmentSlot::Accessory => "장신구",
            EquipmentSlot::Other => "기타",
        }
    }
}

/// 아이템 이름으로 장비 부위 판별
/// - 장신구는 오직 '네크리스'와 '링' 2가지뿐임!
/// - 완드 등 무기류는 절대 장신구로 분류되지 않음.
pub fn get_equipment_slot(name: &str) -> EquipmentSlot {
    if name.contains("네크리스") || name.contains("링") {
        EquipmentSlot::Accessory
    } else if name.contains("갑옷") || name.contains("전투복") || name.contains("상의")
        || name.contains("하의") || name.contains("투구") || name.contains("장갑") || name.contains("신발")
    {
        EquipmentSlot::Armor
    } else if name.contains("소드") || name.contains("부채") || name.contains("코일")
        || name.contains("케인") || name.contains("너클") || name.contains("보우")
        || name.contains("크로스보우") || name.contains("핼버드") || name.contains("완드")
        || name.contains("스태프") || name.contains("단검") || name.contains("대거")
        || name.contains("오브") || name.contains("하프") || name.contains("류트")
    {
        EquipmentSlot::Weapon
    } else {
        EquipmentSlot::Other
    }
}

/// 캐릭터의 현재 직업(job_name)에 따른 1:1 전용 장비 판별
/// - 장신구: 네크리스, 링 2가지만 전 직업 공용 허용
/// - 무기: 각 직업당 오직 고유 무기 딱 1종만 허용
/// - 방어구: 직업 계열에 맞는 중갑(비늘)/가죽/천옷(전투복) 5종만 허용
pub fn is_job_equipment(job_name: &str, item_name: &str) -> bool {
    let slot = get_equipment_slot(item_name);

    // 1. 장신구 (네크리스, 링 2종류만 전 직업 공용)
    if slot == EquipmentSlot::Accessory {
        return item_name.contains("네크리스") || item_name.contains("링");
    }

    let job = job_name.trim();

    // 2. 직업별 1:1 고유 무기 및 방어구 매칭
    // 석궁사수: 라이트 크로스보우 + 가죽 갑옷
    if job.contains("석궁") {
        return match slot {
            EquipmentSlot::Weapon => item_name.contains("라이트 크로스보우"),
            EquipmentSlot::Armor => item_name.contains("가죽 갑옷"),
            _ => false,
        };
    }
    // 장궁병: 라이트 롱보우 + 가죽 갑옷
    if job.contains("장궁") {
        return match slot {
            EquipmentSlot::Weapon => item_name.contains("라이트 롱보우"),
            EquipmentSlot::Armor => item_name.contains("가죽 갑옷"),
            _ => false,
        };
    }
    // 궁수/명사수: 숏보우 + 가죽 갑옷
    if job.contains("궁수") || job.contains("명사수") {
        return match slot {
            EquipmentSlot::Weapon => item_name.contains("숏보우"),
            EquipmentSlot::Armor => item_name.contains("가죽 갑옷"),
            _ => false,
        };
    }

    // 검술사/검사: 론 엣지소드 + 비늘 갑옷
    if job.contains("검술사") || job.contains("검사") {
        return match slot {
            EquipmentSlot::Weapon => item_name.contains("론 엣지소드"),
            EquipmentSlot::Armor => item_name.contains("비늘 갑옷"),
            _ => false,
        };
    }
    // 대검전사: 그레이트 소드 + 비늘 갑옷
    if job.contains("대검") {
        return match slot {
            EquipmentSlot::Weapon => item_name.contains("그레이트 소드"),
            EquipmentSlot::Armor => item_name.contains("비늘 갑옷"),
            _ => false,
        };
    }
    // 기사: 라이트 핼버드 + 비늘 갑옷
    if job.contains("기사") {
        return match slot {
            EquipmentSlot::Weapon => item_name.contains("라이트 핼버드"),
            EquipmentSlot::Armor => item_name.contains("비늘 갑옷"),
            _ => false,
        };
    }
    // 일반 전사: 숏소드 + 비늘 갑옷
    if job.contains("전사") {
        return match slot {
            EquipmentSlot::Weapon => item_name.contains("숏소드"),
            EquipmentSlot::Armor => item_name.contains("비늘 갑옷"),
            _ => false,
        };
    }

    // 듀얼블레이드/쌍검: 샤프 듀얼소드 + 가죽 갑옷
    if job.contains("듀얼") || job.contains("쌍검") {
        return match slot {
            EquipmentSlot::Weapon => item_name.contains("샤프 듀얼소드"),
            EquipmentSlot::Armor => item_name.contains("가죽 갑옷"),
            _ => false,
        };
    }
    // 격투가: 라이트 너클 + 가죽 갑옷
    if job.contains("격투") || job.contains("인파이터") {
        return match slot {
            EquipmentSlot::Weapon => item_name.contains("라이트 너클"),
            EquipmentSlot::Armor => item_name.contains("가죽 갑옷"),
            _ => false,
        };
    }
    // 도적/어쌔신: 서펜트 의식용 단검 + 가죽 갑옷
    if job.contains("도적") || job.contains("어쌔신") {
        return match slot {
            EquipmentSlot::Weapon => item_name.contains("서펜트 의식용 단검"),
            EquipmentSlot::Armor => item_name.contains("가죽 갑옷"),
            _ => false,
        };
    }

    // 댄서: 꽃잎 접부채 + 가죽 갑옷
    if job.contains("댄서") {
        return match slot {
            EquipmentSlot::Weapon => item_name.contains("꽃잎 접부채"),
            EquipmentSlot::Armor => item_name.contains("가죽 갑옷"),
            _ => false,
        };
    }
    // 악사: 커브드 하프 + 가죽 갑옷
    if job.contains("악사") {
        return match slot {
            EquipmentSlot::Weapon => item_name.contains("커브드 하프"),
            EquipmentSlot::Armor => item_name.contains("가죽 갑옷"),
            _ => false,
        };
    }
    // 음유시인: 켈틱 류트 + 가죽 갑옷
    if job.contains("음유") {
        return match slot {
            EquipmentSlot::Weapon => item_name.contains("켈틱 류트"),
            EquipmentSlot::Armor => item_name.contains("가죽 갑옷"),
            _ => false,
        };
    }

    // 빙결술사: 스노우 오브 + 전투복
    if job.contains("빙결") {
        return match slot {
            EquipmentSlot::Weapon => item_name.contains("스노우 오브"),
            EquipmentSlot::Armor => item_name.contains("전투복"),
            _ => false,
        };
    }
    // 화염술사 / 전격술사: 우드 완드 + 전투복
    if job.contains("화염") || job.contains("전격") {
        return match slot {
            EquipmentSlot::Weapon => item_name.contains("우드 완드"),
            EquipmentSlot::Armor => item_name.contains("전투복"),
            _ => false,
        };
    }
    // 일반 마법사: 크리스탈 스태프 + 전투복
    if job.contains("마법") {
        return match slot {
            EquipmentSlot::Weapon => item_name.contains("크리스탈 스태프"),
            EquipmentSlot::Armor => item_name.contains("전투복"),
            _ => false,
        };
    }

    // 힐러: 마블 힐링 완드 + 전투복
    if job.contains("힐러") {
        return match slot {
            EquipmentSlot::Weapon => item_name.contains("마블 힐링 완드"),
            EquipmentSlot::Armor => item_name.contains("전투복"),
            _ => false,
        };
    }
    // 사제: 라운드헤드 케인 + 전투복
    if job.contains("사제") {
        return match slot {
            EquipmentSlot::Weapon => item_name.contains("라운드헤드 케인"),
            EquipmentSlot::Armor => item_name.contains("전투복"),
            _ => false,
        };
    }
    // 수도사: 블런트 쿼터스태프 + 전투복
    if job.contains("수도") {
        return match slot {
            EquipmentSlot::Weapon => item_name.contains("블런트 쿼터스태프"),
            EquipmentSlot::Armor => item_name.contains("전투복"),
            _ => false,
        };
    }
    // 암흑술사: 라운드 코일 + 전투복
    if job.contains("암흑") {
        return match slot {
            EquipmentSlot::Weapon => item_name.contains("라운드 코일"),
            EquipmentSlot::Armor => item_name.contains("전투복"),
            _ => false,
        };
    }

    true
}

/// SQLite DB 파일 저장 경로: .exe 실행 파일과 동일한 이름의 .db 파일 생성 (예: mobi_market.exe -> mobi_market.db)
pub fn get_db_path() -> PathBuf {
    if let Ok(mut exe_path) = std::env::current_exe() {
        exe_path.set_extension("db");
        exe_path
    } else {
        PathBuf::from("mobi_market.db")
    }
}
