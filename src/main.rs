use chrono::{DateTime, NaiveDate, Utc, Weekday};
use clap::Parser;
use dotenvy::dotenv;
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, fmt};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct WorkItem {
    id: u64,
    // url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkItemQueryResult {
    work_items: Vec<WorkItem>,
}

#[derive(Deserialize)]
struct User {
    id: Uuid,
    #[serde(rename = "displayName")]
    display_name: String,
    #[serde(rename = "uniqueName")]
    email: String,
}

impl fmt::Display for User {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} <{}>", self.display_name, self.email)
    }
}

impl fmt::Debug for User {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("User")
            .field("id", &self.id)
            .field("display_name", &self.display_name)
            .field("email", &self.email)
            .finish()
    }
}

#[derive(Debug, Deserialize)]
struct Fields {
    #[serde(rename = "System.ChangedDate")]
    // changed_date: Option<DateTime<Utc>>,
    changed_date: DateTime<Utc>,
    #[serde(rename = "System.ChangedBy")]
    changed_by: User,
    #[serde(rename = "Microsoft.VSTS.Scheduling.CompletedWork")]
    completed_work: Option<f64>,
    #[serde(rename = "System.Title")]
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Revision {
    // id: u32,
    #[allow(dead_code)]
    rev: u32,
    fields: Fields,
}

#[derive(Debug, Deserialize)]
struct Revisions {
    #[allow(dead_code)]
    count: u32,
    value: Vec<Revision>,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
/// Naïve utility to get time logs from Azure Devops
///
/// Playing with way more fun Rust features than needed
struct Args {
    /// First date to include
    #[arg(short, long)]
    from: Option<NaiveDate>,

    /// Last date to include
    #[arg(short, long)]
    to: Option<NaiveDate>,

    /// Email of user
    #[arg(short, long, env = "USERNAME")]
    user: String,

    /// Azuee DevOps access token
    #[arg(long, env = "ACCESS_TOKEN")]
    token: String,

    /// Azuee DevOps Organization
    #[arg(short, long, env = "ORG")]
    organization: String,

    /// Azuee DevOps Project
    #[arg(short, long, env = "PROJECT")]
    project: String,
}

#[tokio::main]
async fn main() -> Result<(), reqwest::Error> {
    dotenv().unwrap();

    // Find dates
    let now = Utc::now();
    let today = now.date_naive();
    let week = today.week(Weekday::Mon);

    let args = Args::parse();
    // eprintln!("{:#?}", args);

    let from = args.from.unwrap_or(week.first_day());
    let to = args.to.unwrap_or(week.last_day());

    eprintln!("From {} to {}", from, to);

    let user = args.user;
    let token = args.token;
    let organization = args.organization;
    let project = args.project;

    let mut map = HashMap::new();
    map.insert(
        "query".to_string(), 
        format!("SELECT [System.Id] FROM workitems WHERE [System.ChangedDate] >= '{from}' AND [System.ChangedDate] <= '{to}' ORDER BY [System.ChangedDate] DESC")
    );
    let client = reqwest::Client::new();
    let query_result: WorkItemQueryResult = client
        .post(format!(
            "https://dev.azure.com/{}/{}/_apis/wit/wiql?api-version=5.1",
            organization, project
        ))
        .basic_auth(&user, Some(&token))
        .json(&map)
        .send()
        .await?
        .json()
        .await?;

    let mut sums: std::collections::BTreeMap<NaiveDate, f64> = std::collections::BTreeMap::new();
    for work_item in query_result.work_items.into_iter() {
        let revisions: Revisions = client
            .get(format!(
                "https://dev.azure.com/{}/{}/_apis/wit/workItems/{}/revisions?api-version=5.0",
                organization, project, work_item.id
            ))
            .basic_auth(&user, Some(&token))
            .send()
            .await?
            .json()
            .await?;

        let mut printed_header = false;
        let mut last_completed_work: f64 = 0.0;
        for revision in revisions.value.into_iter() {
            if let Some(completed_work) = revision.fields.completed_work {
                let diff = completed_work - last_completed_work;
                last_completed_work = completed_work;

                if diff == 0.0 {
                    continue;
                };

                if revision.fields.changed_by.email != user {
                    continue;
                }

                let date = revision.fields.changed_date.date_naive();
                if date < from || date > to {
                    continue;
                }

                if !printed_header {
                    println!(
                        "{} {}",
                        work_item.id,
                        revision.fields.title.unwrap_or("".to_string())
                    );
                    printed_header = true
                }

                sums.entry(date)
                    .and_modify(|sum| *sum += diff)
                    .or_insert(diff);

                println!(
                    "\t{} {} {} {}",
                    date, revision.fields.changed_by, completed_work, diff
                );
            }
        }
    }
    println!("{:#?}", sums);

    Ok(())
}

#[allow(dead_code)]
fn print_work_logs(v: Value) {
    if let Value::Array(revs) = &v["value"] {
        let mut last_completed_work: f64 = 0.0;
        for rev in revs.iter() {
            if let Value::Number(number) = &rev["fields"]["Microsoft.VSTS.Scheduling.CompletedWork"]
            {
                if let Some(completed_work) = number.as_f64() {
                    if last_completed_work == completed_work {
                        continue;
                    };
                    // println!(
                    //     "{}: {} {} <{}> {}, {}",
                    //     rev["rev"],
                    //     rev["fields"]["System.ChangedDate"],
                    //     rev["fields"]["System.ChangedBy"]["displayName"],
                    //     rev["fields"]["System.ChangedBy"]["uniqueName"],
                    //     rev["fields"]["Microsoft.VSTS.Scheduling.CompletedWork"],
                    //     completed_work - last_completed_work
                    // );

                    // Why is this a move?
                    let u: User =
                        serde_json::from_value(rev["fields"]["System.ChangedBy"].clone()).unwrap();
                    eprintln!(
                        "{} {} {}",
                        u,
                        completed_work,
                        completed_work - last_completed_work
                    );

                    last_completed_work = completed_work;
                }
            }
        }
    }
}
