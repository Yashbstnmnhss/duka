use std::{
    any::{Any, TypeId},
    collections::HashMap,
    vec,
};

use anyhow::{Result, anyhow};

pub trait Converter {
    fn from(&self) -> TypeId;
    fn to(&self) -> TypeId;
    fn convert(&self, from: Box<dyn Any>) -> Result<Box<dyn Any>>;
}

pub trait Node {
    fn from(&self) -> TypeId;
    fn to(&self) -> TypeId;
    fn name(&self) -> &'static str;
    fn process(&mut self, input: Box<dyn Any>) -> Result<Box<dyn Any>>;
}

pub struct Pipeline {
    //outs: HashMap<TypeId, Box<dyn Out>>,
    nodes: HashMap<&'static str, (Box<dyn Node>, bool)>,
    converters: HashMap<(TypeId, TypeId), Box<dyn Converter>>,
}
impl Pipeline {
    pub fn new() -> Self {
        Self {
            // pre: vec![],
            //outs: HashMap::new(),
            nodes: HashMap::new(),
            converters: HashMap::new(),
        }
    }
    pub fn converter(mut self, convert: Box<dyn Converter>) -> Self {
        self.converters
            .insert((convert.from(), convert.to()), convert);
        self
    }
    pub fn node(self, node: Box<dyn Node>) -> Self {
        self.node_cond(node, true)
    }
    pub fn node_cond(mut self, node: Box<dyn Node>, enable: bool) -> Self {
        self.nodes.insert(node.name(), (node, enable));
        self
    }

    // pub fn out(mut self, output: Box<dyn Out>) -> Self {
    //     self.outs.insert(output.accept(), output);
    //     self
    // }
    // pub fn pre(mut self, pre_node: Box<dyn Node>) -> Self {
    //     self.pre.push(pre_node);
    //     self
    // }

    pub fn process(&mut self, steps: Vec<&'static str>, mut input: Box<dyn Any>) -> Result<()> {
        let mut type_id: TypeId = (*input).type_id(); // ATTENTION: deref Box<T> to get T's type ID

        for step in steps {
            let (node, enable) = self
                .nodes
                .get_mut(step)
                .ok_or(anyhow!("Cannot found node {}", step))?;
            if !*enable {
                continue;
            }

            let expected_type = node.from();
            if type_id != expected_type {
                let converter = self
                    .converters
                    .get(&(type_id, expected_type))
                    .ok_or(anyhow!("Cannot found suitable converter in {}", step))?;
                input = converter.convert(input)?;
            }
            input = node.process(input)?;
            type_id = node.to();

            // if input.type_id() != type_id {
            //     return Err(anyhow!("Mismatched output from {}", step));
            // }
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct Recipe<A, N = &'static str> {
    line: Vec<RecipePart<A, N>>,
    post: Vec<N>,
    pre: Vec<N>,
}

#[derive(Debug)]
pub struct RecipePart<A, N = &'static str> {
    input: Option<A>,
    output: Option<A>,
    name: N,
    enable: bool,
}
impl<A, N> RecipePart<A, N> {
    pub fn named(name: N) -> Self {
        Self {
            input: None,
            output: None,
            name,
            enable: true,
        }
    }
    pub fn input(mut self, i: A) -> Self {
        self.input = Some(i);
        self
    }
    pub fn output(mut self, o: A) -> Self {
        self.output = Some(o);
        self
    }
    pub fn when(mut self, flag: bool) -> Self {
        self.enable = flag;
        self
    }
}

impl<A: PartialEq, N: Clone> Recipe<A, N> {
    pub fn new() -> Self {
        Self {
            line: vec![],
            post: vec![],
            pre: vec![],
        }
    }
    pub fn step(mut self, p: RecipePart<A, N>) -> Self {
        self.line.push(p);
        self
    }
    pub fn pre(mut self, p: N) -> Self {
        self.pre.push(p);
        self
    }
    pub fn post(mut self, p: N) -> Self {
        self.post.push(p);
        self
    }
    pub fn find(&self, from: A, to: A) -> Result<Vec<N>> {
        for (left_index, part) in self.line.iter().enumerate() {
            if let Some(input) = &part.input
                && from == *input
            {
                for (index, part2) in self.line[left_index..].iter().enumerate() {
                    let right_index = left_index + index;
                    if let Some(output) = &part2.output
                        && to == *output
                    {
                        return Ok(self
                            .pre
                            .clone()
                            .into_iter()
                            .chain(
                                self.line[left_index..(right_index + 1)]
                                    .iter()
                                    .filter_map(|i| i.enable.then_some(i.name.clone()))
                                    .chain(self.post.clone()),
                            )
                            .collect());
                    }
                }
            }
        }
        Err(anyhow!("Failed to find suitable recipe"))
    }
}
