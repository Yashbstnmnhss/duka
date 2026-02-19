//! # Pipeline
//!
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
    line: Vec<RecipePart<A, N>>,
    post: Vec<N>,
    pre: Vec<N>,
}

/// Recipe part definition
#[derive(Debug)]
pub struct RecipePart<A, N = &'static str> {
    input: Option<A>,
    output: Option<A>,
    name: N,
    enable: bool,
}
impl<A, N> RecipePart<A, N> {
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
            line: vec![],
            post: vec![],
            pre: vec![],
        }
    }
}
impl<A: PartialEq + Display, N: Clone> Recipe<A, N> {
    /// Create a step with `RecipePart`
    pub fn step(mut self, part: RecipePart<A, N>) -> Self {
        self.line.push(part);
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
    /// Get the steps between input to output. When there is no such route, it will return `Err`
    pub fn find(&self, from: A, to: A) -> Result<Steps<N>> {
        for (left_index, part) in self.line.iter().enumerate() {
            if let Some(input) = &part.input
                && from == *input
            {
                for (index, part2) in self.line[left_index..].iter().enumerate() {
                    let right_index = left_index + index;
                    if let Some(output) = &part2.output
                        && to == *output
                    {
                        return Ok(Steps {
                            inner: self
                                .pre
                                .clone()
                                .into_iter()
                                .chain(
                                    self.line[left_index..(right_index + 1)]
                                        .iter()
                                        .filter_map(|i| i.enable.then_some(i.name.clone()))
                                        .chain(self.post.clone()),
                                )
                                .collect(),
                        });
                    }
                }
            }
        }
        Err(miette!(
            "Failed to find suitable recipe, from {from} to {to}"
        ))
    }
}
