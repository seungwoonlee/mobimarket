use mobi_market::config::{get_equipment_slot, is_job_equipment, EquipmentSlot};

#[test]
fn test_equipment_slot_classification() {
    // 1. 반지 및 목걸이 (장신구)
    assert_eq!(get_equipment_slot("잔영의 페리도트 네크리스ZZ"), EquipmentSlot::Accessory);
    assert_eq!(get_equipment_slot("잔영의 페리도트 링ZZ"), EquipmentSlot::Accessory);
    assert_eq!(get_equipment_slot("해연의 페리도트 링ZZ"), EquipmentSlot::Accessory);

    // 2. 완드 무기 ("힐링"의 "링"으로 인해 장신구로 오분류되지 않아야 함!)
    assert_eq!(get_equipment_slot("잔영의 마블 힐링 완드ZZ"), EquipmentSlot::Weapon);
    assert_eq!(get_equipment_slot("해연의 마블 힐링 완드ZZ"), EquipmentSlot::Weapon);
    assert_eq!(get_equipment_slot("잔영의 우드 완드ZZ"), EquipmentSlot::Weapon);

    // 3. 방어구 및 무기
    assert_eq!(get_equipment_slot("잔영의 가죽 갑옷 상의ZZ"), EquipmentSlot::Armor);
    assert_eq!(get_equipment_slot("잔영의 샤프 듀얼소드ZZ"), EquipmentSlot::Weapon);
}

#[test]
fn test_dualblade_job_equipment_filtering() {
    let job = "듀얼블레이드";

    // 듀얼블레이드는 오직 샤프 듀얼소드만 무기로 허용되어야 함
    assert!(is_job_equipment(job, "잔영의 샤프 듀얼소드ZZ"));
    assert!(is_job_equipment(job, "해연의 샤프 듀얼소드ZZ"));

    // 힐러 무기인 마블 힐링 완드는 절대 포함되면 안 됨!
    assert!(!is_job_equipment(job, "잔영의 마블 힐링 완드ZZ"));
    assert!(!is_job_equipment(job, "해연의 마블 힐링 완드ZZ"));

    // 전사, 궁수 등 타 직업 무기도 제외되어야 함
    assert!(!is_job_equipment(job, "잔영의 그레이트 소드ZZ"));
    assert!(!is_job_equipment(job, "잔영의 라이트 크로스보우ZZ"));

    // 가죽 갑옷 방어구 및 공용 장신구는 포함되어야 함
    assert!(is_job_equipment(job, "잔영의 가죽 갑옷 상의ZZ"));
    assert!(is_job_equipment(job, "잔영의 페리도트 네크리스ZZ"));
    assert!(is_job_equipment(job, "잔영의 페리도트 링ZZ"));
}

#[test]
fn test_healer_job_equipment_filtering() {
    let job = "힐러";

    // 힐러는 마블 힐링 완드 허용
    assert!(is_job_equipment(job, "잔영의 마블 힐링 완드ZZ"));
    assert!(is_job_equipment(job, "해연의 마블 힐링 완드ZZ"));

    // 듀얼소드는 힐러에게 제외
    assert!(!is_job_equipment(job, "잔영의 샤프 듀얼소드ZZ"));
}
