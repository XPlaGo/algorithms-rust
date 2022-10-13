use std::io::{stdin};

fn main() {
    let tasks: Vec<String> = stdin().lines().map(|line| line.unwrap()).collect();
    for task in tasks {
        
        if check_task(task) {
            print!("YES\n")
        } else {
            print!("NO\n")
        }
    }
}

fn check_task(task: String) -> bool {
    todo!()
}

enum Bracket {
    SIMPLE(),

}

mod fake_collections {
    pub struct Stack<T> {
        vec: Vec<T>
    }

    impl<T> Stack<T> {
        pub const fn new() -> Self {
            Self { vec: Vec::new() }
        }
    }

    impl<T> Stack<T>  {
        pub fn push(&mut self, element: T) {
            self.vec.push(element);
        }

        pub fn pop(&mut self) -> Option<T> {
            self.vec.pop()
        }
    }
}