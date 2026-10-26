# Philosophy of stew

In software teams of every size, it's a basic reality that some people are more familiar with certain parts of the codebase than others. Stew is a tool to make a fine-grained explicit representation of that reality, and to use that representation to gain insight, drive decisions, and facilitate real change in held responsibilities.

## "responsibility"

The meaning of "responsibility" is something that may mean something different to each team.
This tool has no particular opinion of what "responsibility" or "stewardship" means; it just helps you represent it.
Some examples of what responsibility might mean:

- Responsibility can be an intrinsic feeling that arises naturally from creating something and feeling invested in its sustainable continuation.
- Responsibility can be a duty taken on that one grows into.
- Responsibility can be a mandate given by a manager to satisfy some organizational requirements

Stew is ultimately a tool to help make better decisions through improved understanding of where responsibilities actually lie, and to move towards a more desirable sharing of responsibilities.

# Feedback loop

Stew works by facilitating a feedback loop:

1. **Inspect**: gain visibility into current responsibilities
1. **Decide**: make decisions based on responsiblities
2. **Refine**: refine responsibilities based on outcomes

## Inspect

Use stew to get answers to questions like:

- given a diff, who is responsible for which parts?
- how much of the codebase has only one steward? or no steward?
- is anybody holding too much on their own? too little?

## Decide

With the knowledge encoded by stew, you might decide to:

- ask others to give special attention to the parts of a PR diff that they are responsible for, and skim the rest
- refrain from asking for review on a PR if the author is also solely responsible for the affected code
- ask for guidance from the person(s) responsible for a part of code you're working on, and not feel like a bother
- be more exacting in your reviews of code you're responsible for, and not feel like a bother
- ask for someone else to share or take over responsibility on something you're holding, when you have too much to hold
- come up with a plan for adding redundant responsiblity when you notice that too much of the code is held by only one person

## Refine

Looking at the decisions you made, were they good decisions? Does the representation of responsibilities seem true? If not, make some changes to reflect what seems truer.

- when I was asked to review code I'm responsible for, did I understand it well, and was I still able to give detailed feedback? If so, maybe I'm not as responsible as I thought and should remove myself from that part.
- when I skimmed code that I'm not responsible for, or saw code merged in that I didn't review, did I have a lot to say about it? If so maybe I'm more responsible than I thought.
- do I notice that one person's responsibilities just keep growing and growing as they add more and more code? Maybe we need to sow some redundancy into those parts.
