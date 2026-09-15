//! # Pipeline
//!
//! Pipeline crate designed for duka-compilation.
//!

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    fmt::Display,
    hash::Hash,
    vec,
};

use miette::{Result, miette};

/// Converter between two nodes where the type of output from former node is not the same type required by the next node
pub trait Converter {
    fn from(&self) -> TypeId;
    fn to(&self) -> TypeId;
    fn convert(&self, from: Box<dyn Any>) -> Result<Box<dyn Any>>;
}

/// Node, process input and yield output
pub trait Node<N = &'static str> {
    fn from(&self) -> TypeId;
    fn to(&self) -> TypeId;
    fn name(&self) -> N;
    fn process(&mut self, input: Box<dyn Any>) -> Result<Box<dyn Any>>;
}

/// Main pipeline, contains nodes and converters
#[derive(Default)]
pub struct Pipeline<N = &'static str>
where
    N: Eq + Hash,
{
    nodes: HashMap<N, (Box<dyn Node<N>>, bool)>,
    converters: HashMap<(TypeId, TypeId), Box<dyn Converter>>,
}

impl<N: Eq + Hash + Display> Pipeline<N> {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            converters: HashMap::new(),
        }
    }
    pub fn converter(mut self, convert: Box<dyn Converter>) -> Self {
        self.converters
            .insert((convert.from(), convert.to()), convert);
        self
    }
    pub fn node(self, node: Box<dyn Node<N>>) -> Self {
        self.node_cond(node, true)
    }
    pub fn node_cond(mut self, node: Box<dyn Node<N>>, enable: bool) -> Self {
        self.nodes.insert(node.name(), (node, enable));
        self
    }

    pub fn process(&mut self, steps: Steps<N>, mut input: Box<dyn Any>) -> Result<Box<dyn Any>> {
        let mut type_id: TypeId = (*input).type_id(); // ATTENTION: deref Box<T> to get T's type ID

        for step in steps.inner {
            let (node, enable) = self
                .nodes
                .get_mut(&step)
                .ok_or(miette!("Cannot found node named {step}"))?;
            if !*enable {
                continue;
            }

            let expected_type = node.from();
            if type_id != expected_type {
                let converter = self
                    .converters
                    .get(&(type_id, expected_type))
                    .ok_or(miette!("Cannot found suitable converter for {step}"))?;
                input = converter.convert(input)?;
            }
            input = node.process(input)?;
            type_id = node.to();
        }

        Ok(input)
    }
}

/// Steps of processing, created by `Recipe`
#[derive(Debug)]
pub struct Steps<N> {
    inner: Box<[N]>,
}

/// Recipe definition
#[derive(Debug, Default)]
pub struct Recipe<A, N = &'static str> {
    main: Vec<RecipePart<A, N>>,
    subs: Vec<Recipe<A, N>>,
    post: Vec<N>,
    pre: Vec<N>,
}

#[derive(Debug)]
pub enum RecipePart<A, N = &'static str> {
    Step(RecipeStep<A, N>),
    Fork(usize),
}

/// Recipe part definition
#[derive(Debug)]
pub struct RecipeStep<A, N = &'static str> {
    input: Option<A>,
    output: Option<A>,
    name: N,
    enable: bool,
}
impl<A, N> RecipeStep<A, N> {
    /// Create a part with name, there is no input or output type in default
    pub fn named(name: N) -> Self {
        Self {
            input: None,
            output: None,
            name,
            enable: true,
        }
    }
    /// Define the input type of current part
    pub fn input(mut self, i: A) -> Self {
        self.input = Some(i);
        self
    }
    /// Define the output type of current part
    pub fn output(mut self, o: A) -> Self {
        self.output = Some(o);
        self
    }
    /// Define the condition of when to enable this part, with a boolean flag
    pub fn when(mut self, flag: bool) -> Self {
        self.enable = flag;
        self
    }
}

impl<A, N> Recipe<A, N> {
    /// Builder mode, start to build a recipe
    pub fn new() -> Self {
        Self {
            main: vec![],
            post: vec![],
            pre: vec![],
            subs: vec![],
        }
    }
}
impl<A: PartialEq + Display + Clone, N: Clone> Recipe<A, N> {
    /// Create a fork with `Recipe`
    pub fn fork(mut self, recipe: Recipe<A, N>) -> Self {
        self.main.push(RecipePart::Fork(self.subs.len()));
        self.subs.push(recipe);
        self
    }
    /// Create a step with `RecipePart`
    pub fn step(mut self, part: RecipeStep<A, N>) -> Self {
        self.main.push(RecipePart::Step(part));
        self
    }
    /// Declare the common preprocess step. the sooner a part was inserted, the sooner it will be applied
    pub fn pre(mut self, pre: N) -> Self {
        self.pre.push(pre);
        self
    }
    /// Declare the common postprocess step
    pub fn post(mut self, post: N) -> Self {
        self.post.push(post);
        self
    }
    /// Get the steps between input to output. When there is no such route, it will return `None`
    pub fn find(&self, from: A, to: A) -> Option<Steps<N>> {
        let steps = self._find(from, to)?;
        Some(Steps {
            inner: self
                .pre
                .iter()
                .cloned()
                .chain(steps.inner)
                .chain(self.post.clone())
                .collect(),
        })
    }
    // Without pre & post
    fn _find(&self, from: A, to: A) -> Option<Steps<N>> {
        let mut main_forks = vec![];
        let mut collected = vec![];
        for (left_index, part) in self.main.iter().enumerate() {
            match part {
                RecipePart::Fork(to) => {
                    main_forks.push(*to);
                }
                RecipePart::Step(step) => {
                    if !step.enable {
                        continue;
                    }

                    if let Some(input) = &step.input
                        && &from == input
                    {
                        collected.push(step.name.clone());
                        if let Some(output) = &step.output
                            && &to == output
                        {
                            return Some(Steps {
                                inner: collected.into_boxed_slice(),
                            });
                        }
                        let mut current_type = step.output.clone();
                        let mut forks_for_end = vec![];
                        for part2 in self.main[left_index + 1..].iter() {
                            match part2 {
                                RecipePart::Step(step2) => {
                                    if !step2.enable {
                                        continue;
                                    }

                                    collected.push(step2.name.clone());
                                    if let Some(output) = &step2.output {
                                        current_type = Some(output.clone());
                                        if &to == output {
                                            return Some(Steps {
                                                inner: collected.into_boxed_slice(),
                                            });
                                        }
                                    }
                                }
                                RecipePart::Fork(to) => {
                                    if let Some(ref ct) = current_type {
                                        forks_for_end.push((*to, collected.len(), ct.clone()));
                                    }
                                }
                            }
                        }

                        while let Some((idx, len, ct)) = forks_for_end.pop() {
                            collected.truncate(len);
                            if let Some(steps) = self.subs[idx]._find(ct, to.clone()) {
                                return Some(Steps {
                                    inner: collected.into_iter().chain(steps.inner).collect(),
                                });
                            }
                        }

                        collected.clear();
                        continue;
                    }
                }
            }
        }

        main_forks
            .iter()
            .find_map(|f| self.subs[*f]._find(from.clone(), to.clone()))
    }
}
