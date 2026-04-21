(* Expected: 3f 0c 0m (3 functions, 0 classes, 0 methods) *)
(* Adversarial: OCaml let-bindings are functions; let () = is entry point, not a function *)

let top_level x = x * 2

let another_func x y = x + y

let rec factorial n =
  if n <= 1 then 1
  else n * factorial (n - 1)

let () =
  let result = factorial 5 in
  Printf.printf "%d\n" result
