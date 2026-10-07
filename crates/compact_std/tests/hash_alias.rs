use compact_std::prelude::*;
use std::ffi::OsStr;
use std::path::Path;

fn evaluate_repeated_value(evaluations: &std::cell::Cell<usize>) -> u32 {
    evaluations.set(evaluations.get() + 1);
    7
}

fn evaluate_repeat_count(evaluations: &std::cell::Cell<usize>) -> usize {
    evaluations.set(evaluations.get() + 1);
    4
}

fn evaluate_zero_count(evaluations: &std::cell::Cell<usize>) -> usize {
    evaluations.set(evaluations.get() + 1);
    0
}

fn read_evaluation_count(evaluations: &std::cell::Cell<usize>) -> usize {
    evaluations.get()
}

#[test]
fn randomized_hash_aliases_are_available_from_the_prelude() {
    StdArena::with_capacity(4096, |arena| -> Result<()> {
        let mut counts = HashMap::new();
        counts.insert(42_u32, 1_u32, arena)?;
        counts.insert(42, 2, arena)?;
        assert_eq!(counts.get(&42, arena)?, Some(&2));

        let mut ids = HashSet::new();
        assert!(ids.insert(42_u32, arena)?);
        assert!(!ids.insert(42, arena)?);
        assert!(ids.contains(&42, arena)?);
        Ok(())
    })
    .unwrap()
    .unwrap();
}

#[test]
fn compact_os_and_path_aliases_are_available_from_the_prelude() {
    StdArena::with_capacity(4096, |arena| -> Result<()> {
        let mut path = PathBuf::from(Path::new("var/log"), arena)?;
        path.push("service", arena)?;
        assert_eq!(
            path.file_name().unwrap().to_os_string(),
            OsStr::new("service")
        );

        let mut name = OsString::from(OsStr::new("service"), arena)?;
        name.push(OsStr::new(".log"), arena)?;
        assert_eq!(name.to_os_string(), std::ffi::OsString::from("service.log"));
        Ok(())
    })
    .unwrap()
    .unwrap();
}

#[test]
fn format_in_and_arena_format_rewriting_produce_compact_strings() {
    StdArena::with_capacity(4096, |arena| -> Result<()> {
        let direct = format_in!(arena, "job={} status={}", 17_u32, "ready")?;
        assert_eq!(direct.as_str(arena)?, "job=17 status=ready");

        let rewritten = arena!(arena, {
            let name = String::from("worker")?;
            let message = format!("hello {name}")?;
            assert_eq!(message.as_str(arena)?, "hello worker");
            Ok::<_, CollectionError>(message)
        })?;
        assert_eq!(rewritten.as_str(arena)?, "hello worker");
        Ok(())
    })
    .unwrap()
    .unwrap();
}

#[test]
fn format_in_returns_arena_exhaustion() {
    StdArena::with_capacity(compact_std::MIN_ARENA_BYTES, |arena| {
        let result = format_in!(arena, "{}", "x".repeat(128));
        assert!(result.is_err());
    })
    .unwrap();
}

#[test]
fn arena_rewrites_vec_clone_to_string_and_collect_for_compact_types() {
    use std::cell::Cell;

    StdArena::with_capacity(32 * 1024, |arena| -> Result<()> {
        let (
            list,
            empty,
            repeated,
            repeated_strings,
            cloned,
            rendered,
            collected_vec,
            collected_string,
            collected_map,
            collected_set,
            constructed_map_clone,
            name_clone,
            direct_rendered,
            os_string_clone,
            path_clone,
            zero_repeat,
            zero_evaluation_counts,
            evaluation_counts,
        ) = arena!(arena, {
            let list = vec![1_u32, 2, 3];
            let empty: Vec<'_, u32> = vec![];

            let value_evaluations = Cell::new(0);
            let count_evaluations = Cell::new(0);
            let repeated = vec![
                evaluate_repeated_value(&value_evaluations);
                evaluate_repeat_count(&count_evaluations)
            ];
            let evaluation_counts = (
                read_evaluation_count(&value_evaluations),
                read_evaluation_count(&count_evaluations),
            );

            let zero_value_evaluations = Cell::new(0);
            let zero_count_evaluations = Cell::new(0);
            let zero_repeat = vec![
                evaluate_repeated_value(&zero_value_evaluations);
                evaluate_zero_count(&zero_count_evaluations)
            ];
            let zero_evaluation_counts = (
                read_evaluation_count(&zero_value_evaluations),
                read_evaluation_count(&zero_count_evaluations),
            );

            let repeated_strings = vec![String::from("pear")?; 3];
            let cloned = list.clone();
            let name = String::from("compact")?;
            let name_clone = name.clone();
            let rendered = name.to_string();
            let direct_rendered = String::from("direct")?.to_string();
            let os_name = OsString::from(OsStr::new("worker"))?;
            let os_string_clone = os_name.clone();
            let path = PathBuf::from(Path::new("var/log/worker"))?;
            let path_clone = path.clone();

            let collected_vec = [4_u32, 5, 6].into_iter().collect::<Vec<_>>();
            let collected_string = "rust".chars().collect::<String>();
            let collected_map = [(1_u32, 10_u32), (2, 20)]
                .into_iter()
                .collect::<HashMap<_, _>>();
            let collected_set = [8_u32, 9].into_iter().collect::<HashSet<_>>();

            let mut constructed_map = HashMap::with_capacity(1)?;
            constructed_map.insert(3_u32, 30_u32, arena)?;
            let constructed_map_clone = constructed_map.clone();

            Ok::<_, CollectionError>((
                list,
                empty,
                repeated,
                repeated_strings,
                cloned,
                rendered,
                collected_vec,
                collected_string,
                collected_map,
                collected_set,
                constructed_map_clone,
                name_clone,
                direct_rendered,
                os_string_clone,
                path_clone,
                zero_repeat,
                zero_evaluation_counts,
                evaluation_counts,
            ))
        })?;

        assert_eq!(evaluation_counts, (1, 1));
        assert_eq!(zero_evaluation_counts, (1, 1));
        assert_eq!(list.as_slice(arena)?, &[1, 2, 3]);
        assert!(empty.is_empty());
        assert!(zero_repeat.is_empty());
        assert_eq!(repeated.as_slice(arena)?, &[7, 7, 7, 7]);
        assert_eq!(
            repeated_strings.get(2, arena)?.unwrap().as_str(arena)?,
            "pear"
        );
        assert_eq!(cloned.as_slice(arena)?, &[1, 2, 3]);
        assert_eq!(rendered.as_str(arena)?, "compact");
        assert_eq!(name_clone.as_str(arena)?, "compact");
        assert_eq!(direct_rendered.as_str(arena)?, "direct");
        assert_eq!(
            os_string_clone.to_os_string(),
            std::ffi::OsString::from("worker")
        );
        assert_eq!(path_clone.to_path_buf(), Path::new("var/log/worker"));
        assert_eq!(collected_vec.as_slice(arena)?, &[4, 5, 6]);
        assert_eq!(collected_string.as_str(arena)?, "rust");
        assert_eq!(collected_map.get(&2, arena)?, Some(&20));
        assert!(collected_set.contains(&9, arena)?);
        assert_eq!(constructed_map_clone.get(&3, arena)?, Some(&30));

        let (native_clone, native_collected) = arena!(arena, {
            let native: std::vec::Vec<u8> = std::vec![1, 2, 3];
            let native_clone = native.clone();
            let native_collected = native.into_iter().collect::<std::vec::Vec<_>>();
            (native_clone, native_collected)
        });
        assert_eq!(native_clone, [1, 2, 3]);
        assert_eq!(native_collected, [1, 2, 3]);
        Ok(())
    })
    .unwrap()
    .unwrap();
}

#[test]
fn arena_vec_rewrite_propagates_allocation_failure() {
    let result = StdArena::with_capacity(compact_std::MIN_ARENA_BYTES, |arena| -> Result<()> {
        arena!(arena, {
            let _values = vec![0_u8; 1024];
            Ok(())
        })
    });
    assert!(result.unwrap().is_err());
}
